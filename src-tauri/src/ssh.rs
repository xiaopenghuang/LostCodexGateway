//! SSH 隧道管理器：
//! - 参数数组构造（无 shell 拼接、无明文密码）
//! - 启动 System32 ssh.exe 并记录 PID（只清理自己创建的进程）
//! - stderr 流式读取 + 错误分类（DNS/鉴权/Host Key/端口占用/远端禁止转发/掉线）
//! - Host Key 查询（ssh-keygen -F / ssh-keyscan）与首次确认写入

use crate::config::{known_hosts_path, ServerProfile};
use crate::procutil::std_cmd;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

pub const SSH_EXE_DEFAULT: &str = r"C:\Windows\System32\OpenSSH\ssh.exe";
pub const KEYGEN_EXE_DEFAULT: &str = r"C:\Windows\System32\OpenSSH\ssh-keygen.exe";
pub const KEYSCAN_EXE_DEFAULT: &str = r"C:\Windows\System32\OpenSSH\ssh-keyscan.exe";

/// 系统 OpenSSH 检测结果。
///
/// 注意：此前本函数返回裸元组 `(bool, String, String)`，经 Tauri 序列化成
/// JSON 数组 `[true, "C:\\...", "OpenSSH_9.5p1"]`，而前端按对象
/// `{exists, path, version}` 取值，导致 `sshEnv.exists` 恒为 undefined
/// （「服务器」页自动填充 ssh.exe 路径的逻辑从未生效）。改为结构体消除歧义。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SshEnv {
    pub exists: bool,
    pub path: String,
    pub version: String,
}

/// 检测系统 OpenSSH（优先 System32，避免 PATH 中的 Git ssh 干扰）。
pub fn detect_ssh_env() -> SshEnv {
    for cand in [SSH_EXE_DEFAULT.to_string(), resolve_in_path("ssh.exe")] {
        let p = PathBuf::from(&cand);
        if p.exists() {
            if let Ok(out) = std_cmd(&p).arg("-V").output() {
                let version = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return SshEnv {
                    exists: true,
                    path: cand,
                    version,
                };
            }
        }
    }
    SshEnv {
        exists: false,
        path: String::new(),
        version: String::new(),
    }
}

fn resolve_in_path(name: &str) -> String {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(';') {
            let p = PathBuf::from(dir).join(name);
            if p.exists() {
                return p.to_string_lossy().to_string();
            }
        }
    }
    name.to_string()
}

/// 构造 ssh.exe 参数数组（不含任何明文凭据，密钥只传路径）。
///
/// `socks_port` 由调用方传入而非从 `cfg` 读取：本地入口端口是**全局**设置
/// （见 `config::AppSettings`），不属于任何一台服务器——多服务器切换时它必须
/// 保持不变，否则下游客户端指向的地址会失效。
pub fn build_args(cfg: &ServerProfile, socks_port: u16) -> Vec<String> {
    let target = format!("{}@{}", cfg.username.trim(), cfg.host.trim());
    let mut args: Vec<String> = vec![
        "-N".to_string(),
        "-D".to_string(),
        format!("127.0.0.1:{}", socks_port),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=30".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-p".to_string(),
        cfg.port.to_string(),
    ];
    let key = cfg.key_path.trim();
    if !key.is_empty() {
        args.push("-i".to_string());
        args.push(key.to_string());
    }
    args.push(target);
    args
}

/// 错误分类（基于 stderr 文本，不解析密钥内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshErrorClass {
    SshNotFound,
    DnsFailed,
    HostUnreachable,
    AuthFailed,
    HostKeyChanged,
    HostKeyUnknown,
    LocalPortBusy,
    RemoteForwardDenied,
    TunnelDied,
    Other(String),
}

impl std::fmt::Display for SshErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SshNotFound => write!(f, "ssh.exe 不存在"),
            Self::DnsFailed => write!(f, "服务器域名解析失败"),
            Self::HostUnreachable => write!(f, "服务器不可达"),
            Self::AuthFailed => write!(f, "鉴权失败"),
            Self::HostKeyChanged => write!(f, "Host Key 已变化（已阻断）"),
            Self::HostKeyUnknown => write!(f, "Host Key 未确认"),
            Self::LocalPortBusy => write!(f, "本地端口被占用"),
            Self::RemoteForwardDenied => write!(f, "远端禁止 TCP 转发"),
            Self::TunnelDied => write!(f, "隧道已断开"),
            Self::Other(s) => write!(f, "其他错误: {s}"),
        }
    }
}

pub fn classify_stderr(stderr_text: &str) -> SshErrorClass {
    let t = stderr_text.to_lowercase();
    if t.contains("could not resolve hostname") || t.contains("no address associated") {
        SshErrorClass::DnsFailed
    } else if t.contains("cannot assign requested address")
        || t.contains("address already in use")
        || t.contains("cannot listen to port")
        || t.contains("could not request local forwarding")
        || t.contains("bind [")
        || t.contains("bind: ")
    {
        // 端口占用需先于鉴权判定：OpenSSH 端口失败后紧跟 Permission denied 提示
        SshErrorClass::LocalPortBusy
    } else if t.contains("remote host identification has changed")
        || (t.contains("host key verification failed") && t.contains("mismatch"))
    {
        SshErrorClass::HostKeyChanged
    } else if t.contains("host key verification failed") {
        SshErrorClass::HostKeyUnknown
    } else if t.contains("permission denied")
        || t.contains("authentication failed")
        || t.contains("no supported authentication")
    {
        SshErrorClass::AuthFailed
    } else if t.contains("administratively prohibited")
        || (t.contains("open failed") && t.contains("forward"))
    {
        SshErrorClass::RemoteForwardDenied
    } else if t.contains("connection timed out")
        || t.contains("connection refused")
        || t.contains("no route to host")
        || t.contains("network is unreachable")
    {
        SshErrorClass::HostUnreachable
    } else if t.contains("connection closed")
        || t.contains("connection reset")
        || t.contains("broken pipe")
    {
        SshErrorClass::TunnelDied
    } else {
        SshErrorClass::Other(trim_error(&t))
    }
}

fn trim_error(t: &str) -> String {
    let t = t.trim().replace(['\r', '\n'], " ");
    t.chars().take(200).collect()
}

/// 隧道进程句柄：child + stderr 行流 + abort 信号。
pub struct TunnelProcess {
    pub child: Child,
    pub pid: u32,
    pub stderr_rx: mpsc::UnboundedReceiver<String>,
    pub abort_rx: tokio::sync::oneshot::Receiver<()>,
}

/// 启动隧道子进程；返回句柄与 abort 发送端（供断开时立即唤醒监控并终止）。
pub fn spawn_tunnel(
    ssh_exe: &str,
    cfg: &ServerProfile,
    socks_port: u16,
) -> Result<(TunnelProcess, tokio::sync::oneshot::Sender<()>), SshErrorClass> {
    if !PathBuf::from(ssh_exe).exists() {
        return Err(SshErrorClass::SshNotFound);
    }
    let args = build_args(cfg, socks_port);
    let mut cmd = Command::new(ssh_exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW，不弹控制台窗口
    let mut child = cmd.spawn().map_err(|e| SshErrorClass::Other(e.to_string()))?;
    let pid = child.id().unwrap_or(0);
    let (tx, stderr_rx) = mpsc::unbounded_channel();
    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let (abort_tx, abort_rx) = tokio::sync::oneshot::channel();
    Ok((
        TunnelProcess {
            child,
            pid,
            stderr_rx,
            abort_rx,
        },
        abort_tx,
    ))
}

/// 停止隧道进程：只终止我们创建的 PID（永不 taskkill /IM）。
pub fn stop_tunnel(tp: &mut TunnelProcess) {
    let _ = tp.child.start_kill();
}

/// 按 PID 终止进程：只针对本工具记录过的 PID。
/// 先用 tasklist 确认该 PID 仍存在且是我们的 ssh 进程，再打开句柄终止。
pub fn kill_process_by_pid(pid: u32) -> bool {
    let out = std_cmd("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
            // 仅当该 PID 是 ssh.exe 时才终止（我们只会记录自己创建的 ssh PID）
            if text.contains(&pid.to_string()) && text.contains("ssh.exe") {
                #[cfg(windows)]
                {
                    const PROCESS_TERMINATE: u32 = 0x0001;
                    extern "system" {
                        fn OpenProcess(
                            dwDesiredAccess: u32,
                            bInheritHandle: i32,
                            dwProcessId: u32,
                        ) -> *mut std::ffi::c_void;
                        fn TerminateProcess(
                            hProcess: *mut std::ffi::c_void,
                            uExitCode: u32,
                        ) -> i32;
                    }
                    unsafe {
                        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
                        if !h.is_null() {
                            let ok = TerminateProcess(h, 1) != 0;
                            return ok;
                        }
                    }
                }
                false
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// 遗留隧道识别与清理
//
// 为什么需要：状态机是**内存态**，应用一重启就丢。任何「应用不在时留下的
// ssh 进程」——被任务管理器强杀、崩溃、或覆盖安装时安装程序结束旧实例——
// 都会变成应用看不见的**孤儿**：
//
//   - 端口一直被占，环境自检把它误报成「已被其他程序占用」（文案误导）；
//   - 更严重：若 Clash 的 MY-VPS 指向该端口，流量会**静默地继续**从那台
//     服务器出去，而界面上显示「未连接」——用户以为自己已经断了。
//
// `commands::disconnect_with` 救不了这种情况：它只认状态机里记录的那一个
// PID，而重启后状态机是空的。所以必须在启动时主动扫一遍。
// ---------------------------------------------------------------------------

/// 判断一条 ssh 命令行是不是**本工具拉起的隧道**。
///
/// 判据取自 `build_args` 生成的参数组合：用户手敲 ssh 时几乎不可能同时用上
/// `ExitOnForwardFailure` + `ServerAliveInterval=30` + `ServerAliveCountMax=3`
/// + `BatchMode` + `StrictHostKeyChecking` + `ConnectTimeout=15` 这一整套，
/// 再叠加「动态转发端口 == 本工具配置的 SOCKS 端口」，足以与本机其它 ssh 区分。
///
/// 用**逐 token 精确匹配**而非子串包含：`-N` 这类短参数若用 `contains`，
/// 会在路径、用户名、备注等位置误命中。
///
/// **绝不使用 `taskkill /IM ssh.exe`** —— 只处理特征完全匹配的进程。
pub fn is_own_tunnel_cmdline(cmdline: &str, socks_port: u16) -> bool {
    if cmdline.trim().is_empty() {
        return false;
    }
    let has_token = |t: &str| cmdline.split_whitespace().any(|x| x == t);
    has_token("-N")
        && has_token("-D")
        && has_token(&format!("127.0.0.1:{}", socks_port))
        && has_token("ExitOnForwardFailure=yes")
        && has_token("ServerAliveInterval=30")
        && has_token("ServerAliveCountMax=3")
        && has_token("BatchMode=yes")
        && has_token("StrictHostKeyChecking=yes")
        && has_token("ConnectTimeout=15")
}

/// 列出本机全部 ssh.exe 进程的 (PID, 命令行)。
///
/// 用 PowerShell CIM：wmic 在新版 Windows 已弃用（同 `mihomo::from_running_process`
/// 的处理）。命令里**只用单引号**，避免经 Rust 传参时被 Windows 的引号规则二次转义。
///
/// 输出格式：每个进程两行 —— 先 PID，再 CommandLine。命令行不含换行，
/// 因此按两行一组配对是安全的（取不到 CommandLine 时该行为空行，配对仍成立）。
fn list_ssh_processes() -> Vec<(u32, String)> {
    const PS: &str = "Get-CimInstance Win32_Process \
        | Where-Object { $_.Name -eq 'ssh.exe' } \
        | ForEach-Object { $_.ProcessId; $_.CommandLine }";
    let out = match std_cmd("powershell")
        .args(["-NoProfile", "-Command", PS])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if let Ok(pid) = lines[i].trim().parse::<u32>() {
            result.push((pid, lines[i + 1].trim().to_string()));
        }
        i += 2;
    }
    result
}

/// 找出本工具**遗留**的隧道进程 PID。**只读**，不终止任何进程。
pub fn find_stale_tunnel_pids(socks_port: u16) -> Vec<u32> {
    list_ssh_processes()
        .into_iter()
        .filter(|(_, cmd)| is_own_tunnel_cmdline(cmd, socks_port))
        .map(|(pid, _)| pid)
        .collect()
}

/// 终止本工具遗留的隧道进程，返回**实际被终止**的 PID。
///
/// 每个 PID 仍要过一遍 `kill_process_by_pid` 的二次校验（该 PID 此刻仍是
/// ssh.exe）才会动手 —— 延续「只清理本工具创建的进程」这条原则。
pub fn kill_stale_tunnels(socks_port: u16) -> Vec<u32> {
    find_stale_tunnel_pids(socks_port)
        .into_iter()
        .filter(|pid| kill_process_by_pid(*pid))
        .collect()
}

/// 等待进程退出（异步，供状态机收尾）。
pub async fn wait_tunnel(tp: &mut TunnelProcess) {
    let _ = tp.child.wait().await;
}

/// 查询 known_hosts 中是否已有该主机条目；返回 (已知, 指纹文本, key 类型)。
/// 与 OpenSSH 查询语义一致：非 22 端口用 "[host]:port" 格式，也兼容裸 host 条目。
pub fn host_key_known(cfg: &ServerProfile) -> (bool, Option<String>, Option<String>) {
    let kh = known_hosts_path();
    if !kh.exists() {
        return (false, None, None);
    }
    let queries: Vec<String> = if cfg.port != 22 {
        vec![
            format!("[{}]:{}", cfg.host.trim(), cfg.port),
            cfg.host.trim().to_string(),
        ]
    } else {
        vec![cfg.host.trim().to_string()]
    };
    for query in queries {
        let out = std_cmd(KEYGEN_EXE_DEFAULT)
            .args(["-F", &query, "-f", kh.to_string_lossy().as_ref()])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    if line.starts_with('#') || line.trim().is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        // known_hosts 行格式：host keytype base64key [comment]
                        let key_type = parts[1].to_string();
                        let key_b64 = parts[2].to_string();
                        let fp = fingerprint_of(key_type.as_str(), &key_b64);
                        return (true, fp, Some(key_type));
                    }
                }
            }
        }
    }
    (false, None, None)
}

/// 用 ssh-keyscan 拉取服务器指纹（首次确认用）。
pub fn fetch_remote_fingerprint(cfg: &ServerProfile) -> Result<(String, String), String> {
    // 注意：Windows 9.5 的 ssh-keyscan 不支持 -o（无法限定 KEX），
    // 依赖服务器端协商；若服务器只声明 sntrup761 会失败并在错误信息中明确提示。
    let out = std_cmd(KEYSCAN_EXE_DEFAULT)
        .args([
            "-t",
            "ed25519,ecdsa-sha2-nistp256,ssh-rsa",
            "-p",
            &cfg.port.to_string(),
            "-T",
            "8",
            cfg.host.trim(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "ssh-keyscan 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best: Option<(String, String)> = None;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let key_type = parts[1].to_string();
            let key_b64 = parts[2].to_string();
            if let Some(fp) = fingerprint_of(key_type.as_str(), &key_b64) {
                best = Some((key_type, fp));
                if best.as_ref().is_some_and(|(k, _)| k.contains("ed25519")) {
                    break;
                }
            }
        }
    }
    best.ok_or_else(|| "未能从 ssh-keyscan 输出中解析指纹".to_string())
}

/// 计算主机指纹（OpenSSH 标准算法，与 ssh-keygen -lf / ssh.exe 显示一致）。
///
/// known_hosts 与 ssh-keyscan 输出中的 base64 值本身就是公钥的 wire 格式
/// blob（string(keytype) || string(pubkey)），标准指纹 = SHA256(blob)，
/// 以 base64（无 padding）展示，形如 SHA256:xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx。
/// 之前实现错把 SHA256(keytype||0x20||pubkey) 且取错字段（把 host 当 keytype），
/// 导致展示的指纹与 OpenSSH 对不上（实测环境复现）。
pub fn fingerprint_of(_key_type: &str, b64: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = base64_decode(b64)?;
    let digest = Sha256::digest(&bytes);
    Some(format!("SHA256:{}", base64_encode(&digest)))
}

/// 最小 base64 编码（无外部依赖，无 padding，与 OpenSSH 展示一致）。
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out.trim_end_matches('=').to_string()
}

/// 最小 base64 解码（无外部依赖）。
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let v = table.iter().position(|&t| t == c)?;
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push((acc >> bits) as u8);
        }
    }
    Some(buf)
}

/// 确认并写入 known_hosts（只追加刚 keyscan 到的行，写前备份）。
pub fn confirm_host_key(cfg: &ServerProfile) -> Result<(), String> {
    let kh = known_hosts_path();
    let parent = kh
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    if kh.exists() {
        let bak = parent.join(format!(
            "known_hosts.bak_{}",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ));
        std::fs::copy(&kh, &bak).map_err(|e| e.to_string())?;
    }
    let out = std_cmd(KEYSCAN_EXE_DEFAULT)
        .args([
            "-t", "ed25519,ecdsa-sha2-nistp256,ssh-rsa",
            "-p", &cfg.port.to_string(), "-T", "8", cfg.host.trim(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "ssh-keyscan 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines_to_add: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // 关键：ssh 连非 22 端口时查询 "[host]:port" 条目。
    // ssh-keyscan 输出不带端口前缀，这里重写 host 部分以匹配 OpenSSH 查询语义。
    let host_with_port = format!("[{}]:{}", cfg.host.trim(), cfg.port);
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && seen.insert(parts[1].to_string()) {
            lines_to_add.push(format!(
                "{} {} {}",
                host_with_port, parts[1], parts[2]
            ));
        }
    }
    if lines_to_add.is_empty() {
        return Err("ssh-keyscan 未返回任何主机密钥行".to_string());
    }
    let mut content = std::fs::read_to_string(&kh).unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    for l in lines_to_add {
        content.push_str(&l);
        content.push('\n');
    }
    std::fs::write(&kh, content).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_contain_no_plaintext_secrets() {
        let mut cfg = ServerProfile::default();
        cfg.host = "vps.example.com".into();
        cfg.username = "ubuntu".into();
        cfg.key_path = r"C:\Users\me\.ssh\id_ed25519".into();
        let args = build_args(&cfg, 17801);
        let joined = args.join(" ");
        assert!(joined.contains("127.0.0.1:17801"));
        assert!(joined.contains("ubuntu@vps.example.com"));
        assert!(joined.contains("ExitOnForwardFailure=yes"));
        assert!(joined.contains("BatchMode=yes"));
        assert!(joined.contains("StrictHostKeyChecking=yes"));
        assert!(!joined.to_lowercase().contains("password"));
    }

    #[test]
    fn args_order_and_dash_flags() {
        let mut cfg = ServerProfile::default();
        cfg.host = "1.2.3.4".into();
        cfg.username = "root".into();
        cfg.port = 2222;
        let args = build_args(&cfg, 17801);
        assert_eq!(args[0], "-N");
        assert_eq!(args[1], "-D");
        assert_eq!(args[2], "127.0.0.1:17801");
        let p = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[p + 1], "2222");
        assert!(args.last().unwrap().contains("root@1.2.3.4"));
    }

    /// 本地入口端口来自调用方（全局设置），不得从服务器配置读取。
    /// 这钉住「切换服务器时本地端口不变」这一前提。
    #[test]
    fn socks_port_comes_from_caller_not_server_profile() {
        let mut cfg = ServerProfile::default();
        cfg.host = "h".into();
        cfg.username = "u".into();
        let a = build_args(&cfg, 17801);
        let b = build_args(&cfg, 18999);
        assert_eq!(a[2], "127.0.0.1:17801");
        assert_eq!(b[2], "127.0.0.1:18999");
    }

    #[test]
    fn stderr_classification() {
        assert_eq!(
            classify_stderr("ssh: Could not resolve hostname vps.example.com: Name or service not known"),
            SshErrorClass::DnsFailed
        );
        assert_eq!(
            classify_stderr("user@host: Permission denied (publickey)."),
            SshErrorClass::AuthFailed
        );
        assert_eq!(
            classify_stderr("REMOTE HOST IDENTIFICATION HAS CHANGED!"),
            SshErrorClass::HostKeyChanged
        );
        assert_eq!(
            classify_stderr("Host key verification failed."),
            SshErrorClass::HostKeyUnknown
        );
        assert_eq!(
            classify_stderr("bind: Address already in use"),
            SshErrorClass::LocalPortBusy
        );
        assert_eq!(
            classify_stderr("channel 2: open failed: administratively prohibited: open failed"),
            SshErrorClass::RemoteForwardDenied
        );
        assert_eq!(
            classify_stderr("Connection closed by remote host"),
            SshErrorClass::TunnelDied
        );
    }

    #[test]
    fn ssh_env_serializes_as_object_not_tuple() {
        // 回归锁定：detect_ssh_env 必须序列化成 JSON 对象 {exists,path,version}。
        // 历史上返回裸元组 → 前端 sshEnv.exists 恒为 undefined，
        // 「服务器」页自动填充 ssh.exe 路径的逻辑从未生效。
        let env = SshEnv {
            exists: true,
            path: r"C:\Windows\System32\OpenSSH\ssh.exe".to_string(),
            version: "OpenSSH_9.5p1".to_string(),
        };
        let v = serde_json::to_value(&env).unwrap();
        assert!(v.is_object(), "SshEnv 必须序列化为对象而非数组");
        assert_eq!(v.get("exists").and_then(|x| x.as_bool()), Some(true));
        assert!(v.get("path").and_then(|x| x.as_str()).is_some());
        assert!(v.get("version").and_then(|x| x.as_str()).is_some());
    }

    #[test]
    fn fingerprint_format_and_decode() {
        assert_eq!(base64_decode("YWJj"), Some(vec![b'a', b'b', b'c']));
        assert_eq!(base64_encode(b"abc"), "YWJj".to_string());
        // OpenSSH 标准指纹：SHA256("abc") 的 base64（无 padding）
        assert_eq!(
            fingerprint_of("ignored", "YWJj").unwrap(),
            "SHA256:ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0"
        );
        // ed25519 主机密钥回归锁定（合成测试向量，非真实主机密钥）。
        // 目的：钉住「wire blob 直取 SHA256」这一算法语义，防止再退回
        // SHA256(keytype||0x20||pubkey) 的错误实现。
        assert_eq!(
            fingerprint_of(
                "ssh-ed25519",
                "AAAAC3NzaC1lZDI1NTE5AAAAIAABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4f"
            )
            .unwrap(),
            "SHA256:ZkAslGjFiUHdGf/WUL8rQvkib4PTvQatUV0OUQSncCA"
        );
        assert_eq!(base64_decode("!!!invalid"), None);
    }

    // ---- 遗留隧道识别（启动时自扫用）----------------------------------------

    /// 还原一条本工具拉起的隧道命令行（形态与 `build_args` 实际产出一致）。
    fn own_cmdline(socks_port: u16) -> String {
        let mut cfg = ServerProfile::default();
        cfg.host = "vps.example.com".into();
        cfg.username = "ubuntu".into();
        cfg.port = 64824;
        cfg.key_path = r"C:\Users\me\.ssh\id_ed25519".into();
        format!(
            r"C:\Windows\System32\OpenSSH\ssh.exe {}",
            build_args(&cfg, socks_port).join(" ")
        )
    }

    #[test]
    fn recognizes_own_tunnel_cmdline() {
        let port = 17801;
        assert!(
            is_own_tunnel_cmdline(&own_cmdline(port), port),
            "完整的本工具隧道命令行应被认出来"
        );
    }

    /// 端口对不上 → 不认（用户改过 SOCKS 端口，或这是别的工具的隧道）。
    #[test]
    fn rejects_wrong_port() {
        let cmd = own_cmdline(17801);
        assert!(!is_own_tunnel_cmdline(&cmd, 17811));
    }

    /// 用户手敲的 `ssh -D`：端口相同，但没有那一整套 `-o` 参数 → **不认**，
    /// 绝不误杀用户自己的 ssh。
    #[test]
    fn rejects_plain_user_ssh() {
        let cmd = r"C:\Windows\System32\OpenSSH\ssh.exe -N -D 127.0.0.1:17801 me@example.com";
        assert!(!is_own_tunnel_cmdline(cmd, 17801));
    }

    /// 任一特征参数缺失就不认（此处抽掉 ExitOnForwardFailure）。
    #[test]
    fn rejects_when_any_marker_missing() {
        let port = 17801;
        let cmd = own_cmdline(port).replace("ExitOnForwardFailure=yes", "Foo=bar");
        assert!(!is_own_tunnel_cmdline(&cmd, port));
    }

    /// 取不到 CommandLine（空/空白）→ 不认。这是最危险的输入，必须安全侧失败。
    #[test]
    fn rejects_empty_cmdline() {
        assert!(!is_own_tunnel_cmdline("", 17801));
        assert!(!is_own_tunnel_cmdline("   ", 17801));
    }

    /// 回归：必须**逐 token 匹配**而非子串包含。
    /// 路径里含 `-N` 字样，但参数并非本工具那一套 —— 不能因此误判。
    #[test]
    fn token_match_not_substring() {
        let cmd = r"C:\tools\-N-stuff\ssh.exe -D 127.0.0.1:17801 me@example.com";
        assert!(!is_own_tunnel_cmdline(cmd, 17801));
    }
}
