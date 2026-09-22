//! Tauri 命令白名单：前端可调用的全部接口。
//! 无 shell 拼接；所有外部进程都以参数数组启动。

use crate::config::{self, ServerProfile};
use crate::launchers;
use crate::procutil::tokio_cmd;
use crate::ssh::{self, SshErrorClass, TunnelProcess};
use crate::state::{GatewayState, GatewayStateMachine, Inner, LogEntry};
use crate::verify;
use parking_lot::Mutex as PLMutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const SNAPSHOT_EVENT: &str = "gateway://snapshot";
const LOG_EVENT: &str = "gateway://log";

/// 隧道「活跃期」的状态集合：这些状态下有（或即将有）本工具创建的 ssh 子进程。
///
/// 用于判断「现在能不能安全地删服务器 / 改全局端口」以及 UI 的断开按钮。
fn tunnel_is_active(state: GatewayState) -> bool {
    matches!(
        state,
        GatewayState::Connecting
            | GatewayState::TunnelReady
            | GatewayState::EgressVerified
            | GatewayState::Degraded
            | GatewayState::Reconnecting
            | GatewayState::Switching
    )
}

/// 是否有「正在进行中的隧道操作」，此时不应再发起新连接。
///
/// 与 `tunnel_is_active` 的差别只有一处：**`Switching` 不算**。
/// `Switching` 的含义是「旧隧道已经断开、即将用新服务器连上」——
/// 恰恰是 `connect_impl` 期望的入口状态；把它算进去会导致切换永远连不上。
fn connect_in_progress(state: GatewayState) -> bool {
    matches!(
        state,
        GatewayState::Connecting
            | GatewayState::TunnelReady
            | GatewayState::EgressVerified
            | GatewayState::Degraded
            | GatewayState::Reconnecting
            | GatewayState::Disconnecting
    )
}

fn emit_snapshot(app: &AppHandle, inner: &Arc<PLMutex<Inner>>) {
    let snap = inner.lock().snapshot();
    let _ = app.emit(SNAPSHOT_EVENT, snap);
}

fn app_log(app: &AppHandle, inner: &Arc<PLMutex<Inner>>, level: &str, component: &str, msg: impl Into<String>) {
    let msg = msg.into();
    inner.lock().log(level, component, msg.clone());
    let entry = LogEntry {
        ts: chrono::Local::now().format("%H:%M:%S").to_string(),
        level: level.to_string(),
        component: component.to_string(),
        message: msg,
    };
    let _ = app.emit(LOG_EVENT, entry);
}

#[tauri::command]
pub fn get_snapshot(machine: State<'_, GatewayStateMachine>) -> crate::state::GatewaySnapshot {
    machine.snapshot()
}

#[tauri::command]
pub fn detect_ssh_env() -> ssh::SshEnv {
    ssh::detect_ssh_env()
}

/// 保存一台服务器（新增或更新）。
///
/// - `id` 为空 → 新增，并自动设为当前选中（刚加的通常就是想用的那台）
/// - `id` 非空 → 按 id 更新（id 本身不可被外部改写）
#[tauri::command]
pub fn save_server(
    app: AppHandle,
    machine: State<'_, GatewayStateMachine>,
    server: Value,
) -> Result<String, String> {
    let profile: ServerProfile =
        serde_json::from_value(server).map_err(|e| format!("参数解析失败: {}", e))?;
    if profile.host.trim().is_empty() || profile.username.trim().is_empty() {
        return Err("主机地址与用户名不能为空".to_string());
    }
    if profile.port == 0 {
        return Err("SSH 端口不能为 0".to_string());
    }

    let mut cfg = machine.inner.lock().config.clone();
    let msg = if profile.id.trim().is_empty() {
        let id = cfg.add_server(profile);
        cfg.active_server_id = id;
        "已新增服务器并设为当前".to_string()
    } else {
        let name = profile.display_name();
        cfg.update_server(profile)?;
        format!("已保存「{}」", name)
    };
    cfg.normalize();

    let backup = config::save(&cfg).map_err(|e| format!("配置保存失败: {}", e))?;
    machine.inner.lock().config = cfg.clone();
    if cfg.is_configured() && matches!(machine.inner.lock().state, GatewayState::Unconfigured) {
        machine.set_state(GatewayState::Ready);
    }
    emit_snapshot(&app, &machine.inner);
    Ok(match backup {
        Some(b) => format!(
            "{}（旧配置备份于 {}）",
            msg,
            b.file_name().unwrap_or_default().to_string_lossy()
        ),
        None => msg,
    })
}

/// 删除一台服务器。若删的正是当前连接中的服务器，先干净断开再删。
#[tauri::command]
pub async fn delete_server(
    app: AppHandle,
    machine: State<'_, GatewayStateMachine>,
    id: String,
) -> Result<String, String> {
    let (is_active, running) = {
        let inner = machine.inner.lock();
        (
            inner.config.active_server_id == id,
            tunnel_is_active(inner.state),
        )
    };
    if is_active && running {
        disconnect_impl(&app, &machine.inner).await?;
    }

    let mut cfg = machine.inner.lock().config.clone();
    let name = cfg
        .server_by_id(&id)
        .map(|s| s.display_name())
        .unwrap_or_else(|| id.clone());
    if !cfg.remove_server(&id) {
        return Err(format!("找不到服务器 id: {}", id));
    }
    cfg.normalize();
    config::save(&cfg).map_err(|e| format!("配置保存失败: {}", e))?;
    machine.inner.lock().config = cfg.clone();
    if !cfg.is_configured() {
        machine.set_state(GatewayState::Unconfigured);
    }
    emit_snapshot(&app, &machine.inner);
    app_log(&app, &machine.inner, "info", "config", format!("已删除服务器「{}」", name));
    Ok(format!("已删除「{}」", name))
}

/// 单台服务器的只读延迟探测结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerLatency {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub reachable: bool,
    /// TCP 握手耗时（毫秒）。**不是出口延迟**：只反映到服务器 SSH 端口的网络往返。
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// 对全部服务器做只读延迟探测（并发，5 秒超时）。
///
/// 判据是「TCP 能否连上 SSH 端口」，**不建隧道、不认证、不发任何数据**。
/// 它回答的是「哪台服务器网络更近」，不是「哪台出口更快」——两者不是一回事，
/// UI 上必须如实标注，否则用户会拿它当出口延迟来选。
///
/// 返回顺序与配置里的服务器顺序一致，前端可直接按 index 对齐。
#[tauri::command]
pub async fn test_servers(
    machine: State<'_, GatewayStateMachine>,
) -> Result<Vec<ServerLatency>, String> {
    let servers = machine.inner.lock().config.servers.clone();
    let mut handles = Vec::with_capacity(servers.len());
    for s in servers {
        handles.push(tauri::async_runtime::spawn(async move {
            let started = std::time::Instant::now();
            let addr = format!("{}:{}", s.host.trim(), s.port);
            let res = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::TcpStream::connect(&addr),
            )
            .await;
            let (reachable, latency_ms, error) = match res {
                Ok(Ok(_stream)) => (true, Some(started.elapsed().as_millis() as u64), None),
                Ok(Err(e)) => (false, None, Some(format!("{}", e))),
                Err(_) => (false, None, Some("超时（5s）".to_string())),
            };
            ServerLatency {
                id: s.id.clone(),
                name: s.display_name(),
                host: s.host.clone(),
                port: s.port,
                reachable,
                latency_ms,
                error,
            }
        }));
    }
    // 按提交顺序回收：任务本身是并发跑的，这样输出顺序与配置一致且可预测。
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn fetch_host_key(machine: State<'_, GatewayStateMachine>) -> serde_json::Value {
    let cfg = machine.inner.lock().config.clone();
    let server = match cfg.active_server() {
        Some(s) => s.clone(),
        None => {
            return serde_json::json!({
                "known": false, "fingerprint": null, "key_type": null,
                "error": "尚未选择服务器"
            })
        }
    };
    let (known, fp, kt) = ssh::host_key_known(&server);
    if known {
        return serde_json::json!({ "known": true, "fingerprint": fp, "key_type": kt });
    }
    match ssh::fetch_remote_fingerprint(&server) {
        Ok((kt, fp)) => serde_json::json!({ "known": false, "fingerprint": fp, "key_type": kt }),
        Err(e) => {
            serde_json::json!({ "known": false, "fingerprint": null, "key_type": null, "error": e })
        }
    }
}

#[tauri::command]
pub fn confirm_host_key(machine: State<'_, GatewayStateMachine>) -> Result<String, String> {
    let cfg = machine.inner.lock().config.clone();
    let server = cfg
        .active_server()
        .ok_or_else(|| "尚未选择服务器".to_string())?;
    if !server.is_complete() {
        return Err("请先完成服务器配置".to_string());
    }
    ssh::confirm_host_key(server)?;
    Ok("已核对并写入 known_hosts（写前已备份）".to_string())
}

#[tauri::command]
pub async fn test_connection(machine: State<'_, GatewayStateMachine>) -> Result<String, String> {
    let cfg = machine.inner.lock().config.clone();
    let server = cfg
        .active_server()
        .ok_or_else(|| "尚未选择服务器".to_string())?
        .clone();
    if !server.is_complete() {
        return Err("请先完成服务器配置".to_string());
    }
    let exe = pick_ssh_exe(&server)?;
    let target = server.target();
    let mut args = vec![
        "-T".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-p".to_string(),
        server.port.to_string(),
    ];
    if !server.key_path.trim().is_empty() {
        args.push("-i".to_string());
        args.push(server.key_path.trim().to_string());
    }
    args.push(target);
    args.push("exit".to_string());

    let out = tokio_cmd(&exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("无法启动 ssh.exe: {}", e))?;
    if out.status.success() {
        Ok("SSH 连接测试成功（认证通过）".to_string())
    } else {
        let text = String::from_utf8_lossy(&out.stderr);
        Err(format!("SSH 连接测试失败：{}", ssh::classify_stderr(&text)))
    }
}

/// 连接主流程：前置检查 → 启动隧道 → 等待早期错误/确认存活 → 验证出口 → 交给监控任务。
///
/// 与 `connect` / `switch_server` 共用同一实现。**连接哪一台由
/// `config.active_server_id` 决定**——「切换」的实现方式就是「改选中项 + 重连」，
/// 所以这里不需要额外的服务器参数，也就不存在「参数与配置不一致」的可能。
///
/// `during` 是连接阶段对外显示的「进行中」状态：普通连接用 `Connecting`，
/// 切换服务器时全程用 `Switching`。否则用户会看到「断开中 → 连接中」两个阶段，
/// 读起来像点了两次操作，中间那一瞬还可能去点「连接」。
async fn connect_impl(
    app: &AppHandle,
    machine_inner: &Arc<PLMutex<Inner>>,
    during: GatewayState,
) -> Result<String, String> {
    {
        let inner = machine_inner.lock();
        if connect_in_progress(inner.state) {
            return Err("已有连接在进行中，请先断开".to_string());
        }
    }
    let cfg = machine_inner.lock().config.clone();
    if !cfg.is_configured() {
        return Err("请先完成服务器配置".to_string());
    }
    let server = cfg
        .active_server()
        .cloned()
        .ok_or_else(|| "尚未选择服务器".to_string())?;

    // 服务器与两个端口在**连接发起时一次性快照**。后台重连必须回到同一台、
    // 同一端口，不能中途读到被切换改过的配置。generation 已经保证旧监控任务
    // 自杀，这里的快照让「旧任务即便读到也不会连错」不依赖单一机制。
    let socks_port = cfg.socks_port();
    let bridge_port_pref = cfg.bridge_port();

    let exe = pick_ssh_exe(&server)?;
    app_log(app, machine_inner, "info", "ssh", format!("使用 ssh.exe: {}", exe));
    app_log(
        app,
        machine_inner,
        "info",
        "ssh",
        format!("目标服务器: {}（{}）", server.display_name(), server.target()),
    );

    // ① 私钥路径预检（只检查存在性，不读取内容）
    if !server.key_path.trim().is_empty()
        && !std::path::Path::new(server.key_path.trim()).exists()
    {
        return Err(format!("配置的私钥文件不存在: {}", server.key_path.trim()));
    }

    // ② 端口占用预检
    // 注意：这里只是「快速失败」提示，不是互斥保证——检测与 ssh 启动之间存在
    // 时间窗口，另一进程仍可能抢占端口。真正的判定以 ssh 的 bind 失败
    // （ExitOnForwardFailure=yes → stderr → LocalPortBusy）为准，见下方早期错误等待。
    if verify::port_listening(socks_port) {
        let msg = format!("本地端口 {} 已被其他程序占用，请更换端口", socks_port);
        app_log(app, machine_inner, "error", "ssh", msg.clone());
        return Err(msg);
    }

    // ② Host Key 预检
    let (known, _, _) = ssh::host_key_known(&server);
    if !known {
        let msg = "服务器 Host Key 尚未确认：请先在「服务器」页查询并核对指纹后确认";
        app_log(app, machine_inner, "warn", "ssh", msg);
        return Err(msg.to_string());
    }

    machine_inner.lock().state = during;
    emit_snapshot(app, machine_inner);

    // ③ 启动隧道
    let (mut tp, abort_tx) = match ssh::spawn_tunnel(&exe, &server, socks_port) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("隧道启动失败: {}", e);
            {
                let mut inner = machine_inner.lock();
                inner.state = GatewayState::Error;
                inner.last_error = Some(msg.clone());
            }
            emit_snapshot(app, machine_inner);
            return Err(msg);
        }
    };
    let pid = tp.pid;
    {
        let mut inner = machine_inner.lock();
        inner.tunnel_pid = Some(pid);
        inner.tunnel_abort = Some(abort_tx);
    }
    app_log(app, machine_inner, "info", "ssh", format!("SSH 隧道进程已启动 (PID {})", pid));

    // ④ 等待早期错误或确认存活（ConnectTimeout=15 + 余量）
    let mut early_fail: Option<SshErrorClass> = None;
    let mut early_line: Option<String> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(18);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break; // 超时未出错 → 视为存活
        }
        match tokio::time::timeout(remaining, tp.stderr_rx.recv()).await {
            Ok(Some(line)) => {
                let cls = ssh::classify_stderr(&line);
                if is_fatal(&cls) {
                    early_fail = Some(cls);
                    early_line = Some(line);
                    break;
                }
            }
            Ok(None) => {
                early_fail = Some(SshErrorClass::TunnelDied);
                break;
            }
            Err(_) => break,
        }
    }
    if let Some(cls) = early_fail {
        ssh::stop_tunnel(&mut tp);
        ssh::wait_tunnel(&mut tp).await;
        let msg = format!("连接失败: {} {}", cls, early_line.unwrap_or_default());
        app_log(app, machine_inner, "error", "ssh", msg.clone());
        {
            let mut inner = machine_inner.lock();
            inner.state = GatewayState::Error;
            inner.last_error = Some(msg.clone());
            inner.tunnel_pid = None;
            inner.tunnel_abort = None;
        }
        emit_snapshot(app, machine_inner);
        return Err(msg);
    }

    // ⑤ 端口监听确认
    if !verify::port_listening(socks_port) {
        ssh::stop_tunnel(&mut tp);
        ssh::wait_tunnel(&mut tp).await;
        let msg = "隧道进程存活但本地端口未监听（远端可能禁止转发）".to_string();
        {
            let mut inner = machine_inner.lock();
            inner.state = GatewayState::Error;
            inner.last_error = Some(msg.clone());
            inner.tunnel_pid = None;
            inner.tunnel_abort = None;
        }
        emit_snapshot(app, machine_inner);
        return Err(msg);
    }
    machine_inner.lock().state = GatewayState::TunnelReady;
    emit_snapshot(app, machine_inner);
    app_log(
        app,
        machine_inner,
        "info",
        "verify",
        format!("本地 SOCKS 端口 {} 已监听", socks_port),
    );

    // ⑥ 出口验证（含对照出口）
    // 直连探测单端点、5 秒超时：本机直连被限制时不能让「连接」卡 30 秒。
    let direct_timeout = cfg.verify.timeout_secs.min(5);
    let direct =
        verify::direct_egress(&cfg.verify.endpoints, direct_timeout).await;
    // 隧道出口验证：给足时间（隧道出口通常可用），但单个端点失败快速切换
    let mut result =
        verify::verify_tunnel(socks_port, &cfg.verify.endpoints, cfg.verify.timeout_secs).await;
    result.steps.insert(0, direct.clone());
    if !direct.ok {
        // 对照失败不影响隧道验证结论，但记录在案
        app_log(app, machine_inner, "warn", "verify", format!("本机直连对照失败: {}", direct.detail));
    }
    let verified = result.ok;
    if let Some(ip) = result.egress_ip.clone() {
        app_log(app, machine_inner, "info", "verify", format!("隧道出口 IP: {}", ip));
    }
    {
        let mut inner = machine_inner.lock();
        inner.last_verify = Some(result.clone());
        inner.state = if verified {
            GatewayState::EgressVerified
        } else {
            GatewayState::Degraded
        };
    }
    if !verified {
        app_log(app, machine_inner, "warn", "verify", "隧道已通但出口验证失败（降级）");
    }
    emit_snapshot(app, machine_inner);

    // ⑦ 启动 HTTP CONNECT 桥接层（M2 实测：Codex 原生二进制需要 HTTP CONNECT）
    if verified {
        match crate::bridge::start_bridge(socks_port, bridge_port_pref).await {
            Ok((bridge_port, task, stats)) => {
                {
                    let mut inner = machine_inner.lock();
                    inner.bridge_port = Some(bridge_port);
                    inner.bridge_task = Some(task);
                    inner.bridge_stats = Some(stats);
                }
                app_log(
                    app,
                    machine_inner,
                    "info",
                    "bridge",
                    format!("HTTP CONNECT 桥接层已启动: 127.0.0.1:{}", bridge_port),
                );
            }
            Err(e) => {
                app_log(app, machine_inner, "warn", "bridge", format!("桥接层启动失败: {}", e));
            }
        }
    }

    // ⑧ 交给后台监控任务（断线检测 + 可选重连）
    let generation = {
        let mut inner = machine_inner.lock();
        inner.generation += 1;
        inner.generation
    };
    let app_monitor = app.clone();
    let inner_monitor = machine_inner.clone();
    let server_monitor = server.clone();
    tauri::async_runtime::spawn(async move {
        monitor_tunnel(
            app_monitor,
            inner_monitor,
            tp,
            generation,
            server_monitor,
            socks_port,
        )
        .await;
    });

    if verified {
        Ok(format!(
            "连接成功：{}，出口 IP {}",
            server.display_name(),
            result.egress_ip.unwrap_or_default()
        ))
    } else {
        Ok(format!(
            "「{}」隧道已通但出口验证未通过（降级），请查看诊断页",
            server.display_name()
        ))
    }
}

#[tauri::command]
pub async fn connect(
    app: AppHandle,
    machine: State<'_, GatewayStateMachine>,
) -> Result<String, String> {
    connect_impl(&app, &machine.inner, GatewayState::Connecting).await
}

/// 切换到另一台服务器（**硬切**：断开旧隧道 → 改选中项 → 用新服务器重连）。
///
/// 三个刻意的设计选择：
/// - **不做优雅切**（先建新隧道、验证通过再断旧的）。优雅切在单端口模型下做不到
///   ——`socks_port` 是全局固定的，两条隧道不能同时绑同一端口。要支持优雅切就得
///   引入第二组端口，收益不抵复杂度。代价是切换瞬间正在跑的请求会断。
/// - **失败不回滚**。回滚意味着「切到一台坏的，结果又悄悄切回来」，用户看到的现象
///   是「点了切换但没变」，比明确报错更难排查。这里改为如实报错 + 记录
///   `previous_server_id`，前端给一个「切回上一个」按钮，把决定权交回用户。
/// - **未连接时只改选中项**，不触发连接。用户只是浏览/预选服务器时不该被动建隧道。
#[tauri::command]
pub async fn switch_server(
    app: AppHandle,
    machine: State<'_, GatewayStateMachine>,
    id: String,
) -> Result<String, String> {
    let (prev_id, prev_name, target_name) = {
        let inner = machine.inner.lock();
        if inner.state == GatewayState::Switching {
            // 并发切换会互相踩：第二个 switch 会先 disconnect 掉第一个刚建好的隧道。
            // UI 已禁用按钮，这里再兜一层，防止连点或自动化调用。
            return Err("正在切换服务器，请等待当前切换完成".to_string());
        }
        let cfg = &inner.config;
        if cfg.active_server_id == id {
            return Err("这已经是当前服务器".to_string());
        }
        let target = cfg
            .server_by_id(&id)
            .ok_or_else(|| format!("找不到服务器 id: {}", id))?;
        if !target.is_complete() {
            return Err(format!(
                "「{}」配置不完整（缺主机地址或用户名），无法切换",
                target.display_name()
            ));
        }
        let prev = cfg.active_server().map(|s| s.display_name()).unwrap_or_default();
        (cfg.active_server_id.clone(), prev, target.display_name())
    };

    let was_running = {
        let inner = machine.inner.lock();
        tunnel_is_active(inner.state)
    };

    // 先落「切换中」状态，让 UI 立刻有反馈，而不是先卡在断开阶段
    if was_running {
        {
            let mut inner = machine.inner.lock();
            inner.previous_server_id = Some(prev_id.clone());
        }
        app_log(
            &app,
            &machine.inner,
            "info",
            "config",
            format!("正在切换服务器：「{}」→「{}」", prev_name, target_name),
        );

        // 硬切第一步：干净断开（generation 自增，旧监控任务自杀）。
        // 全程停在 Switching，不停回 Ready——避免 UI 显示「断开中 → 连接中」两段。
        disconnect_with(
            &app,
            &machine.inner,
            GatewayState::Switching,
            GatewayState::Switching,
        )
        .await?;
    } else {
        machine.inner.lock().previous_server_id = Some(prev_id.clone());
    }

    // 硬切第二步：改选中项并落盘
    {
        let mut cfg = machine.inner.lock().config.clone();
        cfg.active_server_id = id.clone();
        cfg.normalize();
        config::save(&cfg).map_err(|e| format!("配置保存失败: {}", e))?;
        machine.inner.lock().config = cfg;
    }
    emit_snapshot(&app, &machine.inner);

    if !was_running {
        return Ok(format!("已选中「{}」（当前未连接）", target_name));
    }

    // 硬切第三步：用新服务器重连。失败不回滚，但 previous_server_id 已记录。
    match connect_impl(&app, &machine.inner, GatewayState::Switching).await {
        Ok(msg) => {
            app_log(
                &app,
                &machine.inner,
                "info",
                "config",
                format!("已切换到「{}」：{}", target_name, msg),
            );
            Ok(format!("已切换到「{}」。{}", target_name, msg))
        }
        Err(e) => {
            let msg = format!(
                "已切换到「{}」但连接失败：{}。已选中项仍为「{}」，可点「切回上一个」回到「{}」",
                target_name, e, target_name, prev_name
            );
            app_log(&app, &machine.inner, "error", "config", msg.clone());
            Err(msg)
        }
    }
}

fn pick_ssh_exe(server: &ServerProfile) -> Result<String, String> {
    if !server.ssh_exe_path.trim().is_empty() {
        let p = server.ssh_exe_path.trim().to_string();
        if std::path::Path::new(&p).exists() {
            return Ok(p);
        }
        return Err(format!("配置的 ssh.exe 不存在: {}", p));
    }
    let env = ssh::detect_ssh_env();
    if env.exists {
        Ok(env.path)
    } else {
        Err("未找到系统 ssh.exe".to_string())
    }
}

fn is_fatal(cls: &SshErrorClass) -> bool {
    !matches!(cls, SshErrorClass::Other(_))
}

/// 后台监控：断线检测 + 可选重连（受 generation/abort 约束）。
/// 迭代式：每一轮监控一个隧道进程实例；进程死亡或 abort 后进入重连轮次。
///
/// `server` / `socks_port` 是连接发起时的快照，重连只回这一台、这个端口。
/// 切到别的服务器时 generation 已自增，本任务会在下一次检查点自行退出。
async fn monitor_tunnel(
    app: AppHandle,
    machine_inner: Arc<PLMutex<Inner>>,
    mut tp: TunnelProcess,
    generation: u64,
    server: ServerProfile,
    socks_port: u16,
) {
    loop {
        // 本轮进程监控：任一事件触发即离开本轮。
        // outcome 用 String 承载断线原因（不再用 Box::leak 换 &'static str：
        // 监控循环每轮都会产生新原因，泄漏会随运行时长无限累积）。
        let outcome: Option<String> = tokio::select! {
            _ = &mut tp.abort_rx => Some("手动断开".to_string()),
            line = tp.stderr_rx.recv() => match line {
                Some(l) => {
                    let cls = ssh::classify_stderr(&l);
                    match cls {
                        // 会话仍存活但远端拒绝转发：降级告警，不杀隧道
                        SshErrorClass::RemoteForwardDenied => {
                            {
                                let mut inner = machine_inner.lock();
                                inner.state = GatewayState::Degraded;
                                inner.last_error = Some(
                                    "远端拒绝 TCP 转发（sshd 的 AllowTcpForwarding 可能被禁用）".to_string(),
                                );
                            }
                            emit_snapshot(&app, &machine_inner);
                            app_log(&app, &machine_inner, "warn", "ssh", "远端拒绝 TCP 转发：请确认服务器 sshd 允许转发；本工具不会自动修改服务器配置");
                            continue;
                        }
                        _ if is_fatal(&cls) => Some(format!("{}", cls)),
                        _ => {
                            machine_inner.lock().log("info", "ssh", format!("stderr: {}", truncate(&l)));
                            continue;
                        }
                    }
                }
                None => Some("进程退出".to_string()),
            },
            status = tp.child.wait() => {
                let _ = status;
                Some("进程退出".to_string())
            }
        };
        if machine_inner.lock().generation != generation {
            ssh::stop_tunnel(&mut tp);
            ssh::wait_tunnel(&mut tp).await;
            return;
        }
        let reason = match outcome {
            Some(r) => r,
            None => continue,
        };

        // 断线处理：立刻标记断开、清空出口展示（不悄悄直连）
        {
            let mut inner = machine_inner.lock();
            inner.tunnel_pid = None;
            inner.state = GatewayState::Disconnected;
            inner.last_error = Some(format!("隧道断开: {}", reason));
        }
        emit_snapshot(&app, &machine_inner);
        app_log(&app, &machine_inner, "warn", "ssh", format!("隧道断开: {}", reason));

        // 有限重连（默认关闭）
        let auto_reconnect = machine_inner.lock().config.settings.auto_reconnect;
        let max_attempts = machine_inner.lock().config.settings.max_reconnect_attempts;
        if !auto_reconnect {
            ssh::stop_tunnel(&mut tp);
            ssh::wait_tunnel(&mut tp).await;
            return;
        }
        let mut attempt = 0u32;
        let mut reconnected: Option<TunnelProcess> = None;
        while attempt < max_attempts {
            if machine_inner.lock().generation != generation {
                ssh::stop_tunnel(&mut tp);
                ssh::wait_tunnel(&mut tp).await;
                return;
            }
            machine_inner.lock().state = GatewayState::Reconnecting;
            emit_snapshot(&app, &machine_inner);
            tokio::time::sleep(Duration::from_secs(2u64.pow(attempt.min(6)))).await;
            if machine_inner.lock().generation != generation {
                return;
            }
            let ssh_exe = match pick_ssh_exe(&server) {
                Ok(p) => p,
                Err(e) => {
                    app_log(&app, &machine_inner, "error", "ssh", format!("重连中止：{}", e));
                    break;
                }
            };
            match ssh::spawn_tunnel(&ssh_exe, &server, socks_port) {
                Ok((mut new_tp, new_abort)) => {
                    let new_pid = new_tp.pid;
                    let mut alive = true;
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(18);
                    loop {
                        let remaining =
                            deadline.saturating_duration_since(tokio::time::Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match tokio::time::timeout(remaining, new_tp.stderr_rx.recv()).await {
                            Ok(Some(line)) => {
                                if is_fatal(&ssh::classify_stderr(&line)) {
                                    alive = false;
                                    break;
                                }
                            }
                            Ok(None) => {
                                alive = false;
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    if alive && verify::port_listening(socks_port) {
                        {
                            let mut inner = machine_inner.lock();
                            inner.tunnel_pid = Some(new_pid);
                            inner.tunnel_abort = Some(new_abort);
                            inner.state = GatewayState::EgressVerified;
                        }
                        emit_snapshot(&app, &machine_inner);
                        app_log(
                            &app,
                            &machine_inner,
                            "info",
                            "ssh",
                            format!("重连成功 (PID {})", new_pid),
                        );
                        reconnected = Some(new_tp);
                        break;
                    }
                    ssh::stop_tunnel(&mut new_tp);
                    ssh::wait_tunnel(&mut new_tp).await;
                }
                Err(e) => {
                    app_log(&app, &machine_inner, "warn", "ssh", format!("重连失败: {}", e));
                }
            }
            attempt += 1;
        }
        match reconnected {
            Some(new_tp) => {
                tp = new_tp;
                continue; // 进入下一轮监控
            }
            None => {
                machine_inner.lock().state = GatewayState::Disconnected;
                emit_snapshot(&app, &machine_inner);
                ssh::stop_tunnel(&mut tp);
                ssh::wait_tunnel(&mut tp).await;
                return;
            }
        }
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(160).collect()
}

/// 保存全局设置（本地端口 + 重连策略）。
///
/// 端口是**全局**的：切换服务器时它们不变，Codex CLI 的 `HTTP_PROXY` 因此不用改。
/// 也正因为全局，改动它们会影响所有服务器，所以这里加了硬约束：
/// **隧道活跃期间拒绝改端口**。否则正在跑的桥接层还绑在旧端口上，而配置已经指向
/// 新端口——下游 CLI 会连到一个「配置说有、实际没有」的地址，排查起来极其费劲。
#[tauri::command]
pub fn save_settings(
    machine: State<'_, GatewayStateMachine>,
    socks_port: u16,
    bridge_port: u16,
    auto_reconnect: bool,
    max_reconnect_attempts: u32,
) -> Result<String, String> {
    let (running, old_socks, old_bridge) = {
        let inner = machine.inner.lock();
        (
            tunnel_is_active(inner.state),
            inner.config.socks_port(),
            inner.config.bridge_port(),
        )
    };
    let port_changed = socks_port != old_socks || bridge_port != old_bridge;
    if running && port_changed {
        return Err(
            "隧道正在运行，不能修改本地端口。请先断开连接再改（改完重新连接即可生效）".to_string(),
        );
    }
    if !(1024..=65535).contains(&socks_port) {
        return Err("SOCKS 端口必须在 1024–65535 之间".to_string());
    }
    if !(1024..=65535).contains(&bridge_port) {
        return Err("桥接端口必须在 1024–65535 之间".to_string());
    }
    if socks_port == bridge_port {
        return Err("SOCKS 端口与桥接端口不能相同".to_string());
    }

    let mut cfg = machine.inner.lock().config.clone();
    cfg.settings.socks_port = socks_port;
    cfg.settings.bridge_port = bridge_port;
    cfg.settings.auto_reconnect = auto_reconnect;
    cfg.settings.max_reconnect_attempts = max_reconnect_attempts;
    cfg.normalize();
    config::save(&cfg).map_err(|e| format!("配置保存失败: {}", e))?;
    machine.inner.lock().config = cfg;
    Ok("已保存设置".to_string())
}

/// 断开：generation 自增使监控任务停止并杀掉子进程；只清理本工具创建的进程。
/// 命令包装与托盘「退出」共用同一实现（托盘退出前必须先干净断开）。
///
/// PID 捕获时机很关键：必须在推进 generation / 发 abort 之前把 PID 取出来。
/// 否则监控任务会先一步把 `tunnel_pid` 清空，这里再读就是 None，兜底终止被
/// 跳过；托盘「退出」随后 app.exit(0) 会让 ssh.exe 成为孤儿进程、端口继续
/// 被占用（这正是「退出前自动干净断开隧道」失效的原因）。captured_pid 与
/// 监控任务自清的 tunnel_pid 互不干扰，重复调用 kill_process_by_pid 也是幂等的。
pub async fn disconnect_impl(
    app: &AppHandle,
    machine: &Arc<PLMutex<Inner>>,
) -> Result<String, String> {
    disconnect_with(app, machine, GatewayState::Disconnecting, GatewayState::Ready).await
}

/// `disconnect_impl` 的参数化版本。
///
/// `during` = 断开过程中显示的状态；`after` = 断开完成后落到哪个状态。
/// 切换服务器用 `(Switching, Switching)`：断完不停在 `Ready`，而是继续停在
/// `Switching`，紧接着由 `connect_impl(…, Switching)` 接手——
/// 这样整个切换对外是一个连续的「切换中」，而不是「断开中 → 连接中」两段。
async fn disconnect_with(
    app: &AppHandle,
    machine: &Arc<PLMutex<Inner>>,
    during: GatewayState,
    after: GatewayState,
) -> Result<String, String> {
    let captured_pid = {
        let mut inner = machine.lock();
        // 先捕获本工具记录的 PID（用于最后的兜底终止）
        let captured_pid = inner.tunnel_pid;
        inner.generation += 1;
        inner.state = during;
        if let Some(abort) = inner.tunnel_abort.take() {
            let _ = abort.send(());
        }
        // 停止桥接层任务
        if let Some(task) = inner.bridge_task.take() {
            task.abort();
        }
        inner.bridge_port = None;
        inner.bridge_stats = None;
        captured_pid
    };
    emit_snapshot(app, machine);
    // 等待监控任务响应 abort 并清理（正常路径下它已终止子进程）
    tokio::time::sleep(Duration::from_millis(600)).await;
    {
        let mut inner = machine.lock();
        inner.tunnel_pid = None;
        inner.state = after;
    }
    // 兜底：若监控任务已退出（或从未启动）导致子进程残留，按先前捕获的 PID
    // 直接终止。该 PID 是本工具创建并记录的，绝不 taskkill /IM 影响用户其他 ssh。
    // kill_process_by_pid 内部会先用 tasklist 校验该 PID 仍是 ssh.exe 才动手。
    if let Some(pid) = captured_pid {
        ssh::kill_process_by_pid(pid);
    }
    emit_snapshot(app, machine);
    app_log(app, machine, "info", "ssh", "已断开（仅停止本工具创建的 SSH 进程）");
    Ok("已断开".to_string())
}

#[tauri::command]
pub async fn disconnect(
    app: AppHandle,
    machine: State<'_, GatewayStateMachine>,
) -> Result<String, String> {
    disconnect_impl(&app, &machine.inner).await
}

/// CLI 启动预览：展示将注入的环境变量与代理路径（不实际启动）。
#[tauri::command]
pub fn get_launch_preview(machine: State<'_, GatewayStateMachine>) -> launchers::LaunchPreview {
    let cfg = machine.inner.lock().config.clone();
    let bridge_port = machine.inner.lock().bridge_port;
    launchers::build_preview(&cfg, bridge_port)
}

/// 启动 Codex CLI：仅当隧道处于 EGRESS_VERIFIED 且桥接层运行中。
#[tauri::command]
pub fn launch_codex_cli(
    machine: State<'_, GatewayStateMachine>,
) -> Result<launchers::LaunchResult, String> {
    let cfg = machine.inner.lock().config.clone();
    let bridge_port = machine.inner.lock().bridge_port;
    {
        let inner = machine.inner.lock();
        if inner.state != GatewayState::EgressVerified {
            return Err(format!(
                "隧道未处于出口已验证状态（当前: {:?}），拒绝启动 CLI 以免悄悄直连",
                inner.state
            ));
        }
    }
    let bridge_port = bridge_port.ok_or_else(|| {
        "HTTP CONNECT 桥接层未运行（M2 实测：Codex 需要 HTTP CONNECT，SOCKS 直供无效）".to_string()
    })?;
    launchers::launch_cli(&cfg, bridge_port)
}

/// M3: 只读检测 Mihomo/Clash Verge。
#[tauri::command]
pub fn detect_mihomo() -> crate::mihomo::MihomoDetection {
    crate::mihomo::detect()
}

/// M3: 生成规则片段（仅返回文本，不写任何文件）。
#[tauri::command]
pub fn generate_mihomo_fragment(
    proxy_group: String,
    process_names: Vec<String>,
    process_paths: Vec<String>,
) -> String {
    crate::mihomo::generate_rules_fragment(&proxy_group, &process_names, &process_paths)
}

/// M3: 备份指定文件（用户确认导入前调用）。返回备份路径。
#[tauri::command]
pub fn backup_mihomo_file(path: String) -> Result<String, String> {
    let bak = crate::mihomo::backup_file(&path)?;
    Ok(bak.to_string_lossy().to_string())
}

/// M3: 恢复本工具创建的备份（冲突时拒绝覆盖用户新改动）。
#[tauri::command]
pub fn restore_mihomo_file(path: String, backup: String) -> Result<String, String> {
    crate::mihomo::restore_file(&path, &backup)?;
    Ok("已恢复".to_string())
}

/// 网络诊断：执行完整检测（Mihomo secret 仅内存，可选）。
#[tauri::command]
pub async fn run_diagnostics(
    app: AppHandle,
    machine: State<'_, GatewayStateMachine>,
    mihomo_secret: Option<String>,
) -> Result<crate::diagnostics::DiagReport, String> {
    let report = crate::diagnostics::run_full_diagnostics(&machine.inner, mihomo_secret).await;
    machine.inner.lock().last_diag_report = Some(report.clone());
    app_log(&app, &machine.inner, "info", "diag", format!(
        "诊断完成：隧道 {:?}，出口 {}，耗时 {}ms",
        report.tunnel_status,
        report.egress.gateway_ip.clone().unwrap_or_else(|| "无".to_string()),
        report.duration_ms
    ));
    Ok(report)
}

/// 最近一次诊断报告。
#[tauri::command]
pub fn get_last_diagnostics(
    machine: State<'_, GatewayStateMachine>,
) -> Option<crate::diagnostics::DiagReport> {
    machine.inner.lock().last_diag_report.clone()
}

/// 单独重新检测某个客户端（desktop | cli | ide）。
/// 只做进程发现 + Mihomo 只读关联，不跑完整诊断（避免无谓的 35s 服务器会话与出口探测）。
#[tauri::command]
pub async fn diagnose_client(
    machine: State<'_, GatewayStateMachine>,
    kind: String,
) -> Result<crate::diagnostics::ClientDiag, String> {
    crate::diagnostics::diagnose_single_client(&machine.inner, None, &kind).await
}

/// 配置某台服务器的预期出口 IP（空 = 清除，不校验）。`server_id` 为空则作用于当前选中项。
///
/// 该字段已从全局 `verify` 下移到每台服务器：不同出口的预期 IP 本就不同，
/// 放在全局会导致「切到第二台就被第一台的预期 IP 判为异常」。
#[tauri::command]
pub fn set_expected_egress_ip(
    machine: State<'_, GatewayStateMachine>,
    server_id: String,
    ip: Option<String>,
) -> Result<String, String> {
    let ip = ip.unwrap_or_default().trim().to_string();
    if !ip.is_empty() && !crate::verify::is_ipv4(&ip) && !crate::verify::is_ipv6(&ip) {
        return Err("预期出口 IP 不是合法的 IPv4/IPv6 地址".to_string());
    }
    let mut cfg = machine.inner.lock().config.clone();
    let id = if server_id.trim().is_empty() {
        cfg.active_server_id.clone()
    } else {
        server_id.trim().to_string()
    };
    let name = {
        let slot = cfg
            .servers
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("找不到服务器 id: {}", id))?;
        slot.expected_egress_ip = ip;
        slot.display_name()
    };
    crate::config::save(&cfg).map_err(|e| format!("配置保存失败: {}", e))?;
    machine.inner.lock().config = cfg;
    Ok(format!("已保存「{}」的预期出口 IP", name))
}

/// WSL2 专项检测（只读）：探测 WSL 是否能连到本工具网关。
/// 判据是「在 WSL 内真正建连成功」，不是「装了 WSL」。
#[tauri::command]
pub async fn detect_wsl(
    machine: State<'_, GatewayStateMachine>,
) -> Result<crate::wsl::WslDetection, String> {
    let socks_port = machine.inner.lock().config.socks_port();
    // WSL 探测涉及外部进程调用，放到阻塞线程池，避免卡住 Tauri 的异步运行时
    tauri::async_runtime::spawn_blocking(move || crate::wsl::detect(socks_port))
        .await
        .map_err(|e| format!("WSL 检测任务失败: {}", e))
}

/// 生成 WSL 内注入代理的一次性命令（仅当前 shell 会话，不回写任何配置文件）。
/// 仅在真实探测到可达时允许生成，避免给用户一条注定失败的命令。
#[tauri::command]
pub async fn get_wsl_proxy_command(
    machine: State<'_, GatewayStateMachine>,
    proxy_host: Option<String>,
) -> Result<serde_json::Value, String> {
    let socks_port = machine.inner.lock().config.socks_port();
    let det = tauri::async_runtime::spawn_blocking(move || crate::wsl::detect(socks_port))
        .await
        .map_err(|e| format!("WSL 检测任务失败: {}", e))?;
    if !det.gateway_reachable_from_wsl {
        return Err(format!(
            "WSL 无法连到本工具网关，未生成命令（避免给出注定失败的指引）。原因：{}",
            det.note
        ));
    }
    // 优先使用用户/探测给出的地址；否则用探测到的推荐地址
    let target = proxy_host
        .filter(|h| !h.trim().is_empty())
        .map(|h| h.trim().to_string())
        .or_else(|| {
            det.recommended_proxy
                .as_deref()
                .and_then(|p| p.rsplit_once(':'))
                .map(|(h, _)| h.to_string())
        })
        .ok_or_else(|| "未能确定 WSL 内可用的网关地址".to_string())?;
    let port = det.socks_port;
    Ok(serde_json::json!({
        "host": target,
        "port": port,
        "inject_command": crate::wsl::build_wsl_proxy_command(&target, port),
        "selfcheck_command": crate::wsl::build_wsl_selfcheck_command(&target, port),
        "note": "该命令只影响你粘贴它的那个 shell 会话；本工具不会写入 ~/.bashrc 或 /etc/environment",
    }))
}

/// 开机启动状态（只读，含「条目指向其他程序」的异常标记）。
#[tauri::command]
pub fn get_autostart_status() -> crate::autostart::AutostartStatus {
    crate::autostart::status()
}

/// 启用/关闭开机启动。
/// 只写当前用户注册表 Run 键（无需提权）；关闭时若条目指向其他程序则拒绝删除。
#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<String, String> {
    if enabled {
        crate::autostart::enable()
    } else {
        crate::autostart::disable()
    }
}

#[tauri::command]
pub async fn export_diagnostics(
    machine: State<'_, GatewayStateMachine>,
) -> Result<String, String> {
    let snap = machine.snapshot();
    let mut text = String::new();
    text.push_str("=== LostCodexGateway 脱敏诊断报告 ===\n");
    text.push_str(&format!(
        "生成时间: {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    text.push_str(&format!("状态: {:?}\n", snap.state));
    if let Some(c) = &snap.config {
        match c.active_server() {
            Some(s) => {
                text.push_str(&format!(
                    "当前服务器: {}@{}:{} (名称: {})\n",
                    s.username, s.host, s.port, s.display_name()
                ));
                text.push_str(&format!(
                    "ssh.exe: {}（私钥只存路径，不输出内容）\n",
                    if s.ssh_exe_path.is_empty() {
                        "自动检测"
                    } else {
                        s.ssh_exe_path.as_str()
                    }
                ));
            }
            None => text.push_str("当前服务器: 无\n"),
        }
        text.push_str(&format!("服务器总数: {}\n", c.servers.len()));
        text.push_str(&format!(
            "本地端口: SOCKS {} / HTTP CONNECT 桥接 {}\n",
            c.socks_port(),
            c.bridge_port()
        ));
        if let Some(prev) = &snap.previous_server_id {
            text.push_str(&format!("上次切换前服务器 id: {}\n", prev));
        }
    }
    text.push_str(&format!(
        "SSH 子进程 PID: {}\n",
        snap.ssh_pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "无".into())
    ));
    if let Some(v) = &snap.last_verify {
        text.push_str(&format!(
            "上次出口验证: {} 出口IP={}\n",
            if v.ok { "通过" } else { "未通过" },
            v.egress_ip.clone().unwrap_or_else(|| "无".into())
        ));
        for s in &v.steps {
            text.push_str(&format!(
                "  [{}] {} {} ({})\n",
                if s.ok { "OK" } else { "FAIL" },
                s.timestamp,
                s.label,
                s.detail
            ));
        }
    }
    text.push_str("\n--- 日志（脱敏） ---\n");
    for l in &snap.recent_logs {
        text.push_str(&format!(
            "{} [{}] {} {}\n",
            l.ts,
            l.level.to_uppercase(),
            l.component,
            l.message
        ));
    }
    text.push_str("\n说明：本报告不含私钥内容、Token、请求正文。\n");

    let dir = config::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "diagnostics_{}.txt",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    ));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}
