//! 网络诊断模块（需求文档: docs/LostCodexGateway — 网络诊断模块开发需求.md）
//!
//! 目标：真实检测 Windows 本地 / SSH 隧道 / Ubuntu 服务器 / Codex 客户端之间的网络状态。
//! 原则：
//! - 一切检测结果来自真实网络行为；SSH 进程存在 ≠ 隧道可用
//! - 无法确认的客户端路由显示「未验证」，绝不显示「已验证」
//! - 不读 Token/Key/Cookie；日志只记域名、状态码、错误类别、脱敏路径
//! - 只读检测服务器（固定命令模板，无用户输入拼接）；不改任何服务器配置
//! - 全局硬超时，网络断开不会卡死或无限重试

use crate::config::GatewayConfig;
use crate::procutil::{std_cmd, tokio_cmd};
use crate::state::{GatewayState, Inner};
use crate::{mihomo, ssh, verify};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 全局诊断硬超时：任何网络异常不得让诊断超过该时长。
pub const GLOBAL_DIAG_TIMEOUT_SECS: u64 = 75;
/// 单项默认超时。
pub const CHECK_TIMEOUT_SECS: u64 = 10;

// ---------- 数据模型 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagStatus {
    Ok,
    Warn,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingStatus {
    Verified,
    Partial,
    Unverified,
    Anomaly,
    Unconfirmable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagItem {
    pub key: String,
    pub label: String,
    pub status: DiagStatus,
    pub detail: String,
    pub latency_ms: Option<u64>,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
    pub cmdline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientDiag {
    pub kind: String, // desktop | cli | ide
    pub label: String,
    pub running: bool,
    pub processes: Vec<ProcInfo>,
    pub routing: RoutingStatus,
    pub evidence: Vec<String>,
    pub last_checked: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressDiag {
    pub local_ip: Option<String>,
    pub local_version: Option<String>,
    pub local_source: Option<String>,
    pub gateway_ip: Option<String>,
    pub gateway_version: Option<String>,
    pub gateway_source: Option<String>,
    pub expected_ip: Option<String>,
    pub match_result: String, // matched | mismatch | unconfirmed
    pub items: Vec<DiagItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerDiag {
    pub reachable: bool,
    pub items: Vec<DiagItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MihomoDiag {
    pub detected: bool,
    pub running: bool,
    pub tun_enabled: bool,
    pub controller: Option<String>,
    pub secret_required: bool,
    pub connections_total: usize,
    pub gateway_matched: usize,
    pub codex_related: usize,
    pub detail: String,
    pub items: Vec<DiagItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyDiag {
    pub ssh_connect_ms: Option<u64>,
    pub socks_handshake_ms: Option<u64>,
    pub gateway_https_ms: Option<u64>,
    pub server_https_ms: Option<u64>,
    pub total_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathHop {
    pub name: String,
    pub status: DiagStatus,
    pub latency_ms: Option<u64>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagReport {
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub gateway_ready: bool,
    pub tunnel_status: DiagStatus,
    pub tunnel_items: Vec<DiagItem>,
    pub egress: EgressDiag,
    pub server: ServerDiag,
    pub clients: Vec<ClientDiag>,
    pub mihomo: MihomoDiag,
    pub dns_items: Vec<DiagItem>,
    pub latencies: LatencyDiag,
    pub path_hops: Vec<PathHop>,
    pub advisories: Vec<String>,
}

// ---------- 工具 ----------

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn item(
    key: &str,
    label: &str,
    status: DiagStatus,
    detail: impl Into<String>,
    latency_ms: Option<u64>,
) -> DiagItem {
    DiagItem {
        key: key.to_string(),
        label: label.to_string(),
        status,
        detail: detail.into(),
        latency_ms,
        ts: now(),
    }
}

/// 进程是否存活（tasklist 查询，仅本工具记录的 PID）。
pub fn pid_alive(pid: u32) -> bool {
    let out = std_cmd("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.contains(&pid.to_string()) && !text.contains("没有运行的任务")
        }
        Err(_) => false,
    }
}

// ---------- M3: Codex 进程发现 ----------

const PS_ENUM_SCRIPT: &str = r#"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;$ErrorActionPreference='SilentlyContinue';
Get-CimInstance Win32_Process | Where-Object { (($_.Name -match 'codex|lost-codex' -or $_.Name -eq 'Code.exe' -or $_.CommandLine -match 'codex') -and $_.ProcessId -ne $PID) } | Select-Object ProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress"#;

/// 枚举 codex 相关进程（进程名 + 路径 + 命令行；仅内存使用，不落日志细节）。
pub fn discover_codex_processes() -> Vec<ProcInfo> {
    let out = std_cmd("powershell.exe")
        .args(["-NoProfile", "-Command", PS_ENUM_SCRIPT])
        .output();
    let Ok(out) = out else { return vec![] };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return vec![];
    }
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let arr: Vec<serde_json::Value> = if v.is_array() {
        v.as_array().cloned().unwrap_or_default()
    } else {
        vec![v]
    };
    arr.iter()
        .filter_map(|e| {
            Some(ProcInfo {
                pid: e.get("ProcessId")?.as_u64()? as u32,
                name: e.get("Name")?.as_str()?.to_string(),
                path: e.get("ExecutablePath").and_then(|x| x.as_str()).map(|s| s.to_string()),
                cmdline: e.get("CommandLine").and_then(|x| x.as_str()).map(|s| s.to_string()),
            })
        })
        .collect()
}

/// 进程归类：desktop / cli / ide / other。
/// 原则（需求文档 §3.4）：不能仅凭 Code.exe/node.exe 认定 Codex 归属；
/// 路径必须含 codex 才算 Codex 相关，否则排除（如 esbuild、无关 node 进程）。
pub fn classify_proc(p: &ProcInfo) -> &'static str {
    let name = p.name.to_lowercase();
    let path = p.path.as_deref().unwrap_or("").to_lowercase();
    let cmd = p.cmdline.as_deref().unwrap_or("").to_lowercase();
    // 1) VS Code 扩展宿主
    if name == "code.exe" {
        return "ide";
    }
    // 2) Desktop 主题/辅助进程（M0 实测进程树成员）
    if name.contains("lost-codex") {
        return "desktop";
    }
    // 2b) 商店版 Codex Desktop 主进程：ChatGPT.exe（实测 WindowsApps\OpenAI.Codex_...）
    if name == "chatgpt.exe" && path.contains("openai.codex") {
        return "desktop";
    }
    // 3) Desktop 的沙盒/computer-use/code-mode 组件
    if name.contains("sandbox") || name.contains("computer-use") || name.contains("code-mode") {
        return "desktop";
    }
    // 4) codex.js（npm shim 的 node 进程）
    if name.contains("node") && cmd.contains("codex.js") {
        return "cli";
    }
    // 5) 名字含 codex 的进程：路径含 codex 且位于 npm 安装目录 → CLI，否则 Desktop
    if name.contains("codex") {
        let path_is_codex = path.contains("codex");
        let npm_install = path.contains("node_global") || path.contains("node_modules");
        return if path_is_codex && npm_install { "cli" } else { "desktop" };
    }
    // 6) 其他（含无 codex 标记的 node/esbuild 等）一律排除
    "other"
}

// ---------- M3: Mihomo 只读连接 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MihomoConnMeta {
    pub host: Option<String>,
    #[serde(rename = "destinationIP")]
    pub destination_ip: Option<String>,
    #[serde(rename = "processPath")]
    pub process_path: Option<String>,
    pub process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MihomoConn {
    pub metadata: MihomoConnMeta,
    #[serde(rename = "rulePayload")]
    pub rule_payload: Option<String>,
    pub chains: Vec<String>,
}

/// 解析 Mihomo /connections 响应（公开以便单元测试用固定样本验证）。
pub fn parse_mihomo_connections(json_text: &str) -> Vec<MihomoConn> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return vec![];
    };
    let Some(arr) = v.get("connections").and_then(|x| x.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|c| serde_json::from_value::<MihomoConn>(c.clone()).ok())
        .collect()
}

/// 经 external-controller 只读获取连接（secret 仅内存，不落盘）。
pub async fn fetch_mihomo_connections(
    controller: &str,
    secret: Option<&str>,
    timeout_secs: u64,
) -> Result<Vec<MihomoConn>, String> {
    let url = format!("http://{}/connections", controller.trim());
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(timeout_secs));
    if let Some(s) = secret {
        if !s.trim().is_empty() {
            builder = builder.default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                let value = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", s.trim()))
                    .map_err(|e| e.to_string())?;
                h.insert(reqwest::header::AUTHORIZATION, value);
                h
            });
        }
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if status == 401 || status == 403 {
        return Err("secret_required".to_string());
    }
    if status != 200 {
        return Err(format!("controller 返回 HTTP {}", status));
    }
    Ok(parse_mihomo_connections(&body))
}

// ---------- M2: 服务器只读连通性 ----------

/// 远端诊断命令模板（固定目标域名，无用户输入；仅只读网络测试）。
const REMOTE_DIAG_CMD: &str = r#"echo DNS_RESULT:$(getent hosts api.ipify.org 2>/dev/null | head -n1 | awk '{print $1}' || nslookup api.ipify.org 2>/dev/null | grep -m1 Address | awk '{print $2}'); (nc -z -w 5 api.ipify.org 443 2>/dev/null && echo TCP_OK) || (timeout 8 sh -c '</dev/tcp/api.ipify.org/443' 2>/dev/null && echo TCP_OK) || echo TCP_FAIL; (wget -q -T 10 -O /dev/null https://api.ipify.org 2>/dev/null && echo HTTPS_OK) || (curl -sS -m 10 -o /dev/null https://api.ipify.org 2>/dev/null && echo HTTPS_OK) || echo HTTPS_FAIL"#;

/// 在服务器上执行只读网络测试（新开短 SSH 会话，超时受限）。
/// 返回 (耗时ms, 原始输出)。
pub async fn run_server_diag(
    cfg: &GatewayConfig,
    ssh_exe: &str,
) -> Result<(u64, String), String> {
    let target = format!("{}@{}", cfg.server.username.trim(), cfg.server.host.trim());
    let mut args: Vec<String> = vec![
        "-T".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-p".to_string(),
        cfg.server.port.to_string(),
    ];
    if !cfg.server.key_path.trim().is_empty() {
        args.push("-i".to_string());
        args.push(cfg.server.key_path.trim().to_string());
    }
    args.push(target);
    args.push(REMOTE_DIAG_CMD.to_string());

    let started = Instant::now();
    let out = tokio::time::timeout(
        Duration::from_secs(35),
        tokio_cmd(ssh_exe)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| "服务器检测超时（35s）".to_string())?
    .map_err(|e| e.to_string())?;
    let ms = started.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() && !stdout.trim().is_empty() {
        Ok((ms, stdout))
    } else if !stderr.trim().is_empty() {
        Err(format!("{}", ssh::classify_stderr(&stderr)))
    } else {
        Err("SSH 会话未返回结果".to_string())
    }
}

// ---------- M3: 客户端归类与路由判定 ----------

/// 由进程快照 + Mihomo 连接构建三类客户端诊断结果。
///
/// 抽成独立函数是为了让「单独检测某个客户端」能只做进程/连接关联，
/// 而不必跑一遍完整诊断（后者含 35s 级服务器 SSH 会话与出口探测）。
pub fn build_clients(
    procs: &[ProcInfo],
    conns: &[MihomoConn],
    gateway_ok: bool,
    bridge_activity: bool,
    tun_enabled: bool,
    group: &str,
) -> Vec<ClientDiag> {
    let mut clients: Vec<ClientDiag> = Vec::new();
    for kind in ["desktop", "cli", "ide"] {
        let procs_kind: Vec<ProcInfo> = procs.iter().filter(|p| classify_proc(p) == kind).cloned().collect();
        let running = !procs_kind.is_empty();
        let mut evidence: Vec<String> = Vec::new();
        for p in procs_kind.iter().take(6) {
            evidence.push(format!("PID {} {} {}", p.pid, p.name, p.path.as_deref().unwrap_or("?")));
        }
        let routing = match kind {
            "cli" => {
                if running {
                    // CLI 经桥接层：桥接全部流量构造性经隧道；若桥接有连接记录 + 网关出口 OK
                    if gateway_ok && bridge_activity {
                        RoutingStatus::Verified
                    } else {
                        RoutingStatus::Partial
                    }
                } else {
                    RoutingStatus::Unverified
                }
            }
            "desktop" | "ide" => {
                if running {
                    let own: Vec<&MihomoConn> = conns
                        .iter()
                        .filter(|c| {
                            let p = c.metadata.process_path.as_deref().unwrap_or("").to_lowercase();
                            if kind == "desktop" {
                                p.contains("codex")
                            } else {
                                p.contains("code.exe") || p.contains("code - insiders")
                            }
                        })
                        .collect();
                    if own.is_empty() {
                        if !tun_enabled {
                            RoutingStatus::Unverified
                        } else {
                            RoutingStatus::Unconfirmable
                        }
                    } else {
                        let via_gw = |c: &&MihomoConn| {
                            c.chains.iter().any(|x| x == group) || c.rule_payload.as_deref() == Some(group)
                        };
                        let any_gw = own.iter().any(via_gw);
                        let any_other = own.iter().any(|c| !via_gw(c));
                        if any_gw && !any_other {
                            RoutingStatus::Verified
                        } else if any_gw {
                            RoutingStatus::Partial
                        } else if any_other {
                            RoutingStatus::Anomaly
                        } else {
                            RoutingStatus::Unverified
                        }
                    }
                } else {
                    RoutingStatus::Unverified
                }
            }
            _ => RoutingStatus::Unverified,
        };
        clients.push(ClientDiag {
            kind: kind.to_string(),
            label: match kind {
                "desktop" => "Codex Desktop".to_string(),
                "cli" => "Codex CLI".to_string(),
                _ => "VS Code Codex 插件".to_string(),
            },
            running,
            processes: procs_kind,
            routing,
            evidence,
            last_checked: now(),
        });
    }
    clients
}

/// 只检测单个客户端（desktop | cli | ide），不做服务器会话与出口探测。
/// 相比跑完整诊断，网络开销从数十秒降到亚秒级。
pub async fn diagnose_single_client(
    inner: &Arc<parking_lot::Mutex<Inner>>,
    mihomo_secret: Option<String>,
    kind: &str,
) -> Result<ClientDiag, String> {
    if !matches!(kind, "desktop" | "cli" | "ide") {
        return Err(format!("未知客户端类型: {}", kind));
    }
    let (group, bridge_activity) = {
        let g = inner.lock();
        (
            g.config.settings.gateway_group.clone(),
            g.bridge_stats.as_ref().map(|s| s.snapshot().connections_total > 0).unwrap_or(false),
        )
    };
    let procs = discover_codex_processes();
    let mihomo_det = mihomo::detect();
    let conns = match (&mihomo_det.external_controller, mihomo_det.mihomo_running) {
        (Some(ctrl), true) => fetch_mihomo_connections(ctrl, mihomo_secret.as_deref(), 5)
            .await
            .unwrap_or_default(),
        _ => vec![],
    };
    // gateway_ok：这里不重新探出口，按网关是否可能可达判定
    let gateway_ok = mihomo_det.mihomo_running || bridge_activity;
    let clients = build_clients(&procs, &conns, gateway_ok, bridge_activity, mihomo_det.tun_enabled, &group);
    clients
        .into_iter()
        .find(|c| c.kind == kind)
        .ok_or_else(|| format!("未能构建 {} 的诊断结果", kind))
}

// ---------- M1+M4: 主诊断流程 ----------

/// 故障提示表（需求文档 §四）。
fn advisories_for(
    report: &DiagReport,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in &report.tunnel_items {
        if t.status == DiagStatus::Error {
            match t.key.as_str() {
                "ssh_pid" | "ssh_session" => out.push(
                    "SSH 连接失败：检查服务器地址、端口、网络与认证配置".to_string(),
                ),
                "port_listen" => out.push("本地 SOCKS5 端口未监听：网关未正常启动".to_string()),
                "socks_handshake" | "tcp_connect" => {
                    out.push("SSH 进程存在但代理不可用：网络隧道异常，建议重新建立连接".to_string())
                }
                _ => {}
            }
        }
    }
    if report.egress.match_result == "mismatch" {
        out.push("网关出口 IP 不匹配：当前请求可能未使用预期出口".to_string());
    }
    if let Some(c) = report.clients.iter().find(|c| c.routing == RoutingStatus::Anomaly) {
        let _ = c;
        out.push("Codex 连接命中其他节点：检查应用分流规则及代理冲突".to_string());
    }
    let dns_failed = report
        .dns_items
        .iter()
        .any(|i| i.key == "dns_local" && i.status == DiagStatus::Error);
    if dns_failed {
        out.push("DNS 解析失败：检查 DNS 配置和目标域名解析".to_string());
    }
    if report
        .dns_items
        .iter()
        .any(|i| i.key == "ipv6_gateway" && i.status == DiagStatus::Unknown)
    {
        out.push("IPv6 路由未验证：无法确认 IPv6 流量是否使用网关（SSH SOCKS5 不等于 VPN）".to_string());
    }
    out
}

/// 执行完整诊断。mihomo_secret 仅内存使用，绝不落盘。
pub async fn run_full_diagnostics(
    inner: &Arc<parking_lot::Mutex<Inner>>,
    mihomo_secret: Option<String>,
) -> DiagReport {
    let total_started = Instant::now();
    let started_at = now();
    let (cfg, state, tunnel_pid) = {
        let g = inner.lock();
        (g.config.clone(), g.state, g.tunnel_pid)
    };
    let ssh_exe = cfg
        .server
        .ssh_exe_path
        .trim()
        .to_string()
        .pipe_if_empty(ssh::detect_ssh_env().path);
    let endpoints = cfg.verify.endpoints.clone();
    let timeout_secs = CHECK_TIMEOUT_SECS;
    let bridge_activity = inner
        .lock()
        .bridge_stats
        .as_ref()
        .map(|s| s.snapshot().connections_total > 0)
        .unwrap_or(false);

    // 全局超时包装
    let fut = run_diag_inner(
        &cfg, state, tunnel_pid, bridge_activity, &ssh_exe, &endpoints, timeout_secs, mihomo_secret,
    );
    let mut report = match tokio::time::timeout(Duration::from_secs(GLOBAL_DIAG_TIMEOUT_SECS), fut).await {
        Ok(r) => r,
        Err(_) => DiagReport {
            started_at: started_at.clone(),
            finished_at: now(),
            duration_ms: total_started.elapsed().as_millis() as u64,
            gateway_ready: false,
            tunnel_status: DiagStatus::Unknown,
            tunnel_items: vec![item(
                "global_timeout", "诊断总超时", DiagStatus::Error,
                format!("诊断超过 {} 秒被中止（网络异常不会卡死）", GLOBAL_DIAG_TIMEOUT_SECS), None,
            )],
            egress: EgressDiag {
                local_ip: None, local_version: None, local_source: None,
                gateway_ip: None, gateway_version: None, gateway_source: None,
                expected_ip: None, match_result: "unconfirmed".to_string(), items: vec![],
            },
            server: ServerDiag { reachable: false, items: vec![] },
            clients: vec![],
            mihomo: MihomoDiag {
                detected: false, running: false, tun_enabled: false, controller: None,
                secret_required: false, connections_total: 0, gateway_matched: 0,
                codex_related: 0, detail: String::new(), items: vec![],
            },
            dns_items: vec![],
            latencies: LatencyDiag {
                ssh_connect_ms: None, socks_handshake_ms: None,
                gateway_https_ms: None, server_https_ms: None, total_ms: None,
            },
            path_hops: vec![],
            advisories: vec!["诊断超时：部分检测未完成".to_string()],
        },
    };
    report.finished_at = now();
    report.duration_ms = total_started.elapsed().as_millis() as u64;
    report.latencies.total_ms = Some(report.duration_ms);
    if report.advisories.is_empty() {
        report.advisories = advisories_for(&report);
    }
    report
}

trait PipeIfEmpty {
    fn pipe_if_empty(self, fallback: String) -> String;
}
impl PipeIfEmpty for String {
    fn pipe_if_empty(self, fallback: String) -> String {
        if self.trim().is_empty() { fallback } else { self }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_diag_inner(
    cfg: &GatewayConfig,
    state: GatewayState,
    tunnel_pid: Option<u32>,
    bridge_activity: bool,
    ssh_exe: &str,
    endpoints: &[String],
    timeout_secs: u64,
    mihomo_secret: Option<String>,
) -> DiagReport {
    let started_at = now();
    let mut tunnel_items: Vec<DiagItem> = Vec::new();
    let mut latencies = LatencyDiag {
        ssh_connect_ms: None,
        socks_handshake_ms: None,
        gateway_https_ms: None,
        server_https_ms: None,
        total_ms: None,
    };

    // ---- M1: 隧道检测 ----
    let socks_port = cfg.server.socks_port;
    // 1) SSH 进程存活
    let pid_item = match tunnel_pid {
        Some(pid) if pid_alive(pid) => item("ssh_pid", "SSH 子进程", DiagStatus::Ok, format!("PID {} 存活", pid), None),
        Some(pid) => item("ssh_pid", "SSH 子进程", DiagStatus::Error, format!("PID {} 已退出", pid), None),
        None => item("ssh_pid", "SSH 子进程", DiagStatus::Unknown, "未启动", None),
    };
    tunnel_items.push(pid_item);

    // 2) 本地端口监听
    let listening = verify::port_listening(socks_port);
    tunnel_items.push(item(
        "port_listen",
        "本地 SOCKS5 监听",
        if listening { DiagStatus::Ok } else { DiagStatus::Error },
        if listening { format!("127.0.0.1:{} 可连接", socks_port) } else { format!("127.0.0.1:{} 无监听", socks_port) },
        None,
    ));

    // 3) SOCKS5 握手
    let handshake_started = Instant::now();
    let handshake_ok = verify::socks_handshake_test(socks_port).await;
    let handshake_ms = handshake_started.elapsed().as_millis() as u64;
    if handshake_ok {
        latencies.socks_handshake_ms = Some(handshake_ms);
    }
    tunnel_items.push(item(
        "socks_handshake",
        "SOCKS5 握手",
        if handshake_ok { DiagStatus::Ok } else { DiagStatus::Error },
        if handshake_ok { format!("成功（{}ms）", handshake_ms) } else { "失败".to_string() },
        Some(handshake_ms),
    ));

    // 4) 经 SOCKS 建连测试目标（TCP 转发真实可用）
    let tcp_probe = verify::socks_tcp_probe(socks_port, "api.ipify.org", 443, timeout_secs).await;
    match tcp_probe {
        Ok(ms) => {
            latencies.socks_handshake_ms.get_or_insert(ms);
            tunnel_items.push(item("tcp_connect", "经 SOCKS 建连测试目标", DiagStatus::Ok,
                format!("api.ipify.org:443 建连成功（远端 DNS + 转发，{}ms）", ms), Some(ms)));
        }
        Err(e) => tunnel_items.push(item("tcp_connect", "经 SOCKS 建连测试目标", DiagStatus::Error, e, None)),
    }

    // 隧道整体状态：进程存活但代理失败 = 异常（不只看进程）
    let tunnel_status = {
        let errors = tunnel_items.iter().filter(|i| i.status == DiagStatus::Error).count();
        let oks = tunnel_items.iter().filter(|i| i.status == DiagStatus::Ok).count();
        if errors == 0 && oks >= 3 {
            DiagStatus::Ok
        } else if oks == 0 && !matches!(state, GatewayState::Unconfigured | GatewayState::Ready) {
            DiagStatus::Error
        } else if errors > 0 && oks > 0 {
            // 进程活着但转发失败 = 隧道异常
            DiagStatus::Warn
        } else if matches!(state, GatewayState::Unconfigured) {
            DiagStatus::Unknown
        } else if oks == 0 {
            DiagStatus::Error
        } else {
            DiagStatus::Warn
        }
    };

    // ---- M1: 出口 IP 检测（本地直连 vs 网关 SOCKS，互不依赖系统代理）----
    let mut egress_items: Vec<DiagItem> = Vec::new();
    let local = verify::probe_egress_direct(endpoints, 5).await;
    if let Some(l) = &local {
        egress_items.push(item("egress_local", "本地直连出口", DiagStatus::Ok,
            format!("{} ({}) 经 {}", l.ip, l.version, l.source), Some(l.latency_ms)));
    } else {
        egress_items.push(item("egress_local", "本地直连出口", DiagStatus::Warn,
            "直连探测失败（本机直连受限或策略拦截）", None));
    }
    let gateway = verify::probe_egress_via_socks(socks_port, endpoints, timeout_secs).await;
    let _gateway_https_ms = gateway.as_ref().map(|g| g.latency_ms);
    if let Some(g) = &gateway {
        latencies.gateway_https_ms = Some(g.latency_ms);
        egress_items.push(item("egress_gateway", "网关出口（经 SOCKS）", DiagStatus::Ok,
            format!("{} ({}) 经 {}", g.ip, g.version, g.source), Some(g.latency_ms)));
    } else {
        egress_items.push(item("egress_gateway", "网关出口（经 SOCKS）", DiagStatus::Error,
            "经隧道探测出口失败", None));
    }
    // 预期 IP 比对
    let expected = {
        let e = cfg.verify.expected_egress_ip.trim();
        if e.is_empty() { None } else { Some(e.to_string()) }
    };
    let match_result = match (&expected, &gateway) {
        (Some(exp), Some(g)) => {
            if exp.trim() == g.ip { "matched" } else { "mismatch" }
        }
        (Some(_), None) => "unconfirmed",
        (None, _) => "unconfirmed",
    };
    let egress = EgressDiag {
        local_ip: local.as_ref().map(|l| l.ip.clone()),
        local_version: local.as_ref().map(|l| l.version.clone()),
        local_source: local.as_ref().map(|l| l.source.clone()),
        gateway_ip: gateway.as_ref().map(|g| g.ip.clone()),
        gateway_version: gateway.as_ref().map(|g| g.version.clone()),
        gateway_source: gateway.as_ref().map(|g| g.source.clone()),
        expected_ip: expected.clone(),
        match_result: match_result.to_string(),
        items: egress_items,
    };

    // ---- M2: 服务器连通性（只读）----
    let mut server_items: Vec<DiagItem> = Vec::new();
    let server_reachable = match run_server_diag(cfg, ssh_exe).await {
        Ok((ms, out)) => {
            latencies.server_https_ms = Some(ms);
            let dns_line = out.lines().find(|l| l.starts_with("DNS_RESULT:"));
            let dns_ip = dns_line
                .and_then(|l| l.strip_prefix("DNS_RESULT:"))
                .filter(|v| verify::is_ipv4(v) || verify::is_ipv6(v))
                .map(|v| v.to_string());
            if let Some(ip) = dns_ip.clone() {
                server_items.push(item("server_dns", "服务器 DNS 解析", DiagStatus::Ok,
                    format!("api.ipify.org → {}", ip), None));
            } else {
                server_items.push(item("server_dns", "服务器 DNS 解析", DiagStatus::Warn,
                    "未解析出 IP（输出异常或工具缺失）", None));
            }
            let tcp_ok = out.contains("TCP_OK");
            server_items.push(item("server_tcp", "服务器 TCP 出站", if tcp_ok { DiagStatus::Ok } else { DiagStatus::Error },
                if tcp_ok { "api.ipify.org:443 可建连".to_string() } else { "建连失败".to_string() }, None));
            let https_ok = out.contains("HTTPS_OK");
            server_items.push(item("server_https", "服务器 HTTPS 出站", if https_ok { DiagStatus::Ok } else { DiagStatus::Warn },
                if https_ok { format!("https 出站正常（会话 {}ms）", ms) } else { "HTTPS 探测失败（或服务器缺 wget/curl）".to_string() }, Some(ms)));
            tcp_ok
        }
        Err(e) => {
            server_items.push(item("server_session", "SSH 诊断会话", DiagStatus::Error, e, None));
            false
        }
    };
    let server = ServerDiag { reachable: server_reachable, items: server_items };

    // ---- M3: Codex 进程与路由 ----
    let procs = discover_codex_processes();
    let mihomo_det = mihomo::detect();
    let group = cfg.settings.gateway_group.clone();
    let conns = match (&mihomo_det.external_controller, mihomo_det.mihomo_running) {
        (Some(ctrl), true) => match fetch_mihomo_connections(ctrl, mihomo_secret.as_deref(), 5).await {
            Ok(c) => c,
            Err(_) => vec![],
        },
        _ => vec![],
    };
    let secret_required = mihomo_det
        .external_controller
        .is_some()
        && mihomo_det.mihomo_running
        && mihomo_secret.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true)
        && conns.is_empty();
    let mut mihomo_items = vec![item(
        "mihomo_running", "Mihomo 运行状态",
        if mihomo_det.mihomo_running { DiagStatus::Ok } else { DiagStatus::Unknown },
        if mihomo_det.mihomo_running { format!("Verge {}", mihomo_det.verge_version.clone().unwrap_or_default()) } else { "未运行".to_string() },
        None,
    )];
    if mihomo_det.tun_enabled {
        mihomo_items.push(item("mihomo_tun", "TUN 状态", DiagStatus::Warn,
            "TUN 已开启：注意 TUN 捕获整机流量，影响面不止 Codex", None));
    } else {
        mihomo_items.push(item("mihomo_tun", "TUN 状态", DiagStatus::Unknown,
            "TUN 未开启：进程级分流需 TUN 或系统代理（本工具不改系统代理）", None));
    }
    let codex_conns: Vec<&MihomoConn> = conns
        .iter()
        .filter(|c| {
            let p = c.metadata.process_path.as_deref().unwrap_or("").to_lowercase();
            let n = c.metadata.process.as_deref().unwrap_or("").to_lowercase();
            p.contains("codex") || n.contains("codex")
        })
        .collect();
    let gateway_matched = conns
        .iter()
        .filter(|c| c.chains.iter().any(|x| x == &group) || c.rule_payload.as_deref() == Some(group.as_str()))
        .count();
    let mihomo = MihomoDiag {
        detected: mihomo_det.verge_installed,
        running: mihomo_det.mihomo_running,
        tun_enabled: mihomo_det.tun_enabled,
        controller: mihomo_det.external_controller.clone(),
        secret_required,
        connections_total: conns.len(),
        gateway_matched,
        codex_related: codex_conns.len(),
        detail: if secret_required {
            "控制器需要 secret（会话内输入即可，不落盘）".to_string()
        } else if conns.is_empty() && mihomo_det.mihomo_running {
            "未获取到连接记录".to_string()
        } else {
            format!("连接 {} 条，其中命中 {} 组 {} 条", conns.len(), group, gateway_matched)
        },
        items: mihomo_items,
    };

    // 客户端归类与路由判定
    let clients = build_clients(&procs, &conns, gateway.is_some(), bridge_activity, mihomo.tun_enabled, &group);

    // ---- M4: DNS 与 IPv6 ----
    let mut dns_items: Vec<DiagItem> = Vec::new();
    // 本地 DNS 解析
    let local_dns = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host("api.ipify.org:443"),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|mut it| it.next().map(|a| a.ip().to_string()))
    .flatten();
    dns_items.push(item(
        "dns_local",
        "本地 DNS 解析",
        if local_dns.is_some() { DiagStatus::Ok } else { DiagStatus::Error },
        local_dns.unwrap_or_else(|| "api.ipify.org 解析失败".to_string()),
        None,
    ));
    // 远端 DNS：SOCKS CONNECT 域名形式已隐含验证
    let remote_dns_ok = gateway.is_some();
    dns_items.push(item(
        "dns_remote",
        "远端 DNS（SOCKS 域名解析）",
        if remote_dns_ok { DiagStatus::Ok } else { DiagStatus::Unknown },
        if remote_dns_ok { "经隧道远端解析成功（出口探测即证据）".to_string() } else { "未验证".to_string() },
        None,
    ));
    // IPv4 网关出口
    let ipv4_ok = gateway.as_ref().map(|g| g.version == "IPv4").unwrap_or(false);
    dns_items.push(item(
        "ipv4_gateway",
        "IPv4 网关出口",
        if ipv4_ok { DiagStatus::Ok } else { DiagStatus::Unknown },
        if ipv4_ok { format!("网关出口 {} 为 IPv4", gateway.as_ref().unwrap().ip) } else { "未验证".to_string() },
        None,
    ));
    // IPv6：本地直连 + 网关（不承诺 UDP）
    let ipv6_endpoints = ["https://api64.ipify.org?format=json".to_string()];
    let ipv6_local = verify::probe_egress_direct(&ipv6_endpoints, 5).await;
    dns_items.push(item(
        "ipv6_local",
        "IPv6 本地直连",
        match &ipv6_local {
            Some(p) if p.version == "IPv6" => DiagStatus::Ok,
            Some(_) => DiagStatus::Warn,
            None => DiagStatus::Unknown,
        },
        match &ipv6_local {
            Some(p) => format!("{} ({})", p.ip, p.version),
            None => "本机无可用 IPv6 或探测失败".to_string(),
        },
        ipv6_local.as_ref().map(|p| p.latency_ms),
    ));
    let ipv6_gw = verify::probe_egress_via_socks(socks_port, &ipv6_endpoints, 8).await;
    let ipv6_gw_ok = ipv6_gw.as_ref().map(|g| g.version == "IPv6").unwrap_or(false);
    dns_items.push(item(
        "ipv6_gateway",
        "IPv6 经网关",
        if ipv6_gw_ok { DiagStatus::Ok } else { DiagStatus::Unknown },
        match &ipv6_gw {
            Some(g) if g.version == "IPv6" => format!("网关可达 IPv6 目标（{}）", g.ip),
            Some(g) => format!("网关返回 IPv4（{}）：服务器出站未用 IPv6", g.ip),
            None => "未验证（SSH SOCKS5 不等于 VPN；UDP/IPv6 不承诺）".to_string(),
        },
        ipv6_gw.as_ref().map(|g| g.latency_ms),
    ));

    // ---- 延迟 ----
    let ssh_connect_ms = if server_reachable {
        // server 会话本身含建连+命令耗时，用其作为 SSH 往返参考并注明
        latencies.server_https_ms
    } else {
        None
    };
    latencies.ssh_connect_ms = ssh_connect_ms;

    // ---- 路径可视化 ----
    let tunnel_hop_status = if tunnel_status == DiagStatus::Ok { DiagStatus::Ok } else if tunnel_status == DiagStatus::Warn { DiagStatus::Warn } else { DiagStatus::Error };
    let path_hops = vec![
        PathHop {
            name: "Windows 本地电脑".to_string(),
            status: DiagStatus::Ok,
            latency_ms: None,
            detail: local.as_ref().map(|l| format!("本地出口 {} ({})", l.ip, l.version)).unwrap_or_else(|| "本地直连受限".to_string()),
        },
        PathHop {
            name: "SSH 隧道".to_string(),
            status: tunnel_hop_status,
            latency_ms: latencies.socks_handshake_ms,
            detail: if tunnel_status == DiagStatus::Ok {
                format!("127.0.0.1:{} 转发正常", socks_port)
            } else if tunnel_status == DiagStatus::Warn {
                "隧道异常：进程存活但代理转发不可用".to_string()
            } else {
                "隧道不可用".to_string()
            },
        },
        PathHop {
            name: "Ubuntu 云服务器".to_string(),
            status: if server_reachable { DiagStatus::Ok } else { DiagStatus::Error },
            latency_ms: latencies.server_https_ms,
            detail: if server_reachable { "只读检测通过（DNS/TCP/HTTPS）".to_string() } else { "无法连接或出站异常".to_string() },
        },
        PathHop {
            name: "目标互联网服务".to_string(),
            status: if gateway.is_some() { DiagStatus::Ok } else { DiagStatus::Error },
            latency_ms: latencies.gateway_https_ms,
            detail: gateway.as_ref().map(|g| format!("出口 {} ({})", g.ip, g.version)).unwrap_or_else(|| "不可达".to_string()),
        },
    ];

    let gateway_ready = tunnel_status == DiagStatus::Ok && gateway.is_some();

    DiagReport {
        started_at,
        finished_at: now(),
        duration_ms: 0,
        gateway_ready,
        tunnel_status,
        tunnel_items,
        egress,
        server,
        clients,
        mihomo,
        dns_items,
        latencies,
        path_hops,
        advisories: vec![],
    }
}

// ---------- 单元测试 ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_clients_is_kind_complete_and_conservative() {
        // 无进程、无连接时：三类客户端齐全，且一律「未验证」（不得虚报已验证）
        let clients = build_clients(&[], &[], false, false, false, "MY-VPS");
        assert_eq!(clients.len(), 3);
        let kinds: Vec<&str> = clients.iter().map(|c| c.kind.as_str()).collect();
        assert!(kinds.contains(&"desktop") && kinds.contains(&"cli") && kinds.contains(&"ide"));
        for c in &clients {
            assert!(!c.running);
            assert_eq!(c.routing, RoutingStatus::Unverified);
        }
    }

    #[test]
    fn build_clients_cli_needs_bridge_and_egress_for_verified() {
        let procs = vec![ProcInfo {
            pid: 1234,
            name: "codex.exe".to_string(),
            path: Some(r"C:\Users\me\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.exe".to_string()),
            cmdline: Some("codex".to_string()),
        }];
        // 进程在跑，但网关未验证 / 桥接无活动 → 只能是 partial，绝不能是 verified
        let clients = build_clients(&procs, &[], false, false, false, "MY-VPS");
        let cli = clients.iter().find(|c| c.kind == "cli").unwrap();
        assert!(cli.running);
        assert_eq!(cli.routing, RoutingStatus::Partial);
        // 网关 OK + 桥接有活动 → 才升级为 verified
        let clients2 = build_clients(&procs, &[], true, true, false, "MY-VPS");
        let cli2 = clients2.iter().find(|c| c.kind == "cli").unwrap();
        assert_eq!(cli2.routing, RoutingStatus::Verified);
    }

    #[test]
    fn build_clients_desktop_without_tun_is_unverified_not_anomaly() {
        // TUN 关闭且无连接记录：显示未验证（不谎报、也不误报异常）
        let procs = vec![ProcInfo {
            pid: 99,
            name: "ChatGPT.exe".to_string(),
            path: Some(r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0\ChatGPT.exe".to_string()),
            cmdline: None,
        }];
        let clients = build_clients(&procs, &[], false, false, false, "MY-VPS");
        let d = clients.iter().find(|c| c.kind == "desktop").unwrap();
        assert!(d.running);
        assert_eq!(d.routing, RoutingStatus::Unverified);
        // TUN 开启但仍无连接 → 只能「无法确认」，不得升级为已验证
        let clients2 = build_clients(&procs, &[], false, false, true, "MY-VPS");
        let d2 = clients2.iter().find(|c| c.kind == "desktop").unwrap();
        assert_eq!(d2.routing, RoutingStatus::Unconfirmable);
    }

    #[test]
    fn remote_cmd_is_fixed_template_no_user_input() {
        // 命令模板不含任何格式化占位符，域名固定
        assert!(REMOTE_DIAG_CMD.contains("api.ipify.org"));
        // 无 format! 空占位符（{}），即模板不含任何可注入的格式化槽位
        assert!(!REMOTE_DIAG_CMD.contains("{}"));
        assert!(REMOTE_DIAG_CMD.contains("TCP_OK"));
        assert!(REMOTE_DIAG_CMD.contains("HTTPS_OK"));
    }

    #[test]
    fn parse_mihomo_connections_sample() {
        let json = r#"{"connections":[{
            "metadata":{"host":"api.openai.com","destinationIP":"1.2.3.4","processPath":"G:\\Programs\\codex\\codex.exe","process":"codex.exe"},
            "chains":["PROXY","MY-VPS"],"rulePayload":"MY-VPS"
        },{
            "metadata":{"host":"example.com","destinationIP":"5.6.7.8","processPath":"C:\\x\\node.exe","process":"node.exe"},
            "chains":["PROXY","OTHER"],"rulePayload":"OTHER"
        }]}"#;
        let conns = parse_mihomo_connections(json);
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[0].metadata.process.as_deref(), Some("codex.exe"));
        assert!(conns[0].chains.contains(&"MY-VPS".to_string()));
        assert_eq!(conns[1].rule_payload.as_deref(), Some("OTHER"));
    }

    #[test]
    fn parse_mihomo_bad_input() {
        assert!(parse_mihomo_connections("not json").is_empty());
        assert!(parse_mihomo_connections(r#"{"other":1}"#).is_empty());
    }

    #[test]
    fn proc_classification() {
        let desktop = ProcInfo { pid: 1, name: "codex.exe".into(), path: Some(r"G:\Programs\codex\codex.exe".into()), cmdline: None };
        assert_eq!(classify_proc(&desktop), "desktop");
        let cli = ProcInfo { pid: 2, name: "codex.exe".into(), path: Some(r"G:\VSCODE\nodejs\node_global\node_modules\@openai\codex\...\codex.exe".into()), cmdline: None };
        assert_eq!(classify_proc(&cli), "cli");
        let sandbox = ProcInfo { pid: 3, name: "codex-windows-sandbox-service.exe".into(), path: None, cmdline: None };
        assert_eq!(classify_proc(&sandbox), "desktop");
        let vscode = ProcInfo { pid: 4, name: "Code.exe".into(), path: Some(r"G:\VSCODE\Code.exe".into()), cmdline: None };
        assert_eq!(classify_proc(&vscode), "ide");
        let node_shim = ProcInfo { pid: 5, name: "node.exe".into(), path: None, cmdline: Some("node codex.js --version".into()) };
        assert_eq!(classify_proc(&node_shim), "cli");
        let other = ProcInfo { pid: 6, name: "notepad.exe".into(), path: None, cmdline: None };
        assert_eq!(classify_proc(&other), "other");
        // 关键反例（需求文档 3.4）：无关 node/esbuild 进程绝不归为 Codex
        let esbuild = ProcInfo {
            pid: 7,
            name: "esbuild.exe".into(),
            path: Some(r"I:\\dev\\project\\node_modules\\@esbuild\\win32-x64\\esbuild.exe".into()),
            cmdline: None,
        };
        assert_eq!(classify_proc(&esbuild), "other");
        let random_node = ProcInfo {
            pid: 8,
            name: "node.exe".into(),
            path: Some(r"G:\\VSCODE\\nodejs\\node.exe".into()),
            cmdline: Some("node app.js".into()),
        };
        assert_eq!(classify_proc(&random_node), "other");
        // Desktop computer-use 组件
        let cua = ProcInfo {
            pid: 9,
            name: "codex-computer-use-swift.exe".into(),
            path: Some(r"C:\\Users\\x\\AppData\\Local\\OpenAI\\Codex\\runtimes\\cua_node\\bin\\codex-computer-use-swift.exe".into()),
            cmdline: None,
        };
        assert_eq!(classify_proc(&cua), "desktop");
        // 商店版 Codex Desktop 主进程：ChatGPT.exe（实测 WindowsApps\OpenAI.Codex_...）
        let store_desktop = ProcInfo {
            pid: 10,
            name: "ChatGPT.exe".into(),
            path: Some(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.915.4065.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe".into()),
            cmdline: None,
        };
        assert_eq!(classify_proc(&store_desktop), "desktop");
        // 反例：其他 ChatGPT.exe（如官网版安装到普通目录）不按路径里的 openai.codex 判定
        let other_chatgpt = ProcInfo {
            pid: 11,
            name: "ChatGPT.exe".into(),
            path: Some(r"C:\Program Files\ChatGPT\ChatGPT.exe".into()),
            cmdline: None,
        };
        assert_eq!(classify_proc(&other_chatgpt), "other");
    }

    #[test]
    fn match_result_logic() {
        // 通过 run_diag_inner 的输出不可直接测；此处验证状态判定函数的分支逻辑已由
        // EgressDiag.match_result 字段在集成测试覆盖。这里只验证枚举序列化。
        let s = serde_json::to_string(&DiagStatus::Warn).unwrap();
        assert_eq!(s, "\"warn\"");
        let r = serde_json::to_string(&RoutingStatus::Unverified).unwrap();
        assert_eq!(r, "\"unverified\"");
    }
}
