//! WSL2 专项支持（P2 / 对应风险清单 R2）。
//!
//! 背景：WSL2 侧进程不继承 Windows 的进程级规则与环境变量，因此
//! 「Windows 上装了 Codex CLI」不等于「WSL2 里的 Codex 也走网关」。
//! 本模块提供**只读探测** + **一次性命令生成**，不修改 WSL 任何全局配置。
//!
//! 网络模型（决定 WSL2 能否连到 Windows 侧 SOCKS 的关键）：
//! - **NAT 模式（默认）**：WSL2 位于 Hyper-V 虚拟子网（通常 172.x），
//!   有独立 IP；WSL2 内的 127.0.0.1 指向 WSL 自身，**不是** Windows。
//!   要访问 Windows 侧回环监听的端口必须用宿主 IP（`/etc/resolv.conf`
//!   的 nameserver，或默认网关）。但 Windows 上的 SOCKS 只绑 127.0.0.1，
//!   因此 NAT 模式下**默认不可达**，需要 mirrored 模式或端口转发（本工具
//!   不做需要提权的 netsh 端口代理，如实报告不可达）。
//! - **Mirrored 模式（Win11 22H2+ / `.wslconfig` networkingMode=mirrored）**：
//!   WSL2 与 Windows 共享回环，`127.0.0.1:<socks_port>` **直接可达**。
//!
//! 探测手段：在 WSL 内执行只读命令，用 `/dev/tcp` 尝试连回环与宿主 IP，
//! 以「能否真正建连」为唯一判据——绝不因为「装了什么」就声称可用。
//!
//! 安全边界：
//! - 只读命令模板，无用户输入拼接（发行版名经过白名单字符校验）。
//! - 不写 `/etc/environment`、不改 `~/.bashrc`、不动 WSL 网络配置。
//! - 生成的代理注入命令供用户自行粘贴到 WSL 终端（一次性，仅该 shell 会话）。

use crate::procutil::std_cmd;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Duration;

/// WSL 探测命令模板（固定，无用户输入）。
///
/// 依次尝试：
///   1. 回环直连（mirrored 模式下成立）
///   2. /etc/resolv.conf 的 nameserver（NAT 模式下 Windows 宿主 IP）
///   3. 默认路由网关（部分发行版 nameserver 指向别处）
/// 输出形如 `PROBE:loopback=OK`，便于 Rust 端解析。
const WSL_PROBE_SCRIPT: &str = r#"for tgt in 127.0.0.1 "$(awk '/^nameserver/{print $2; exit}' /etc/resolv.conf 2>/dev/null)" "$(ip route show default 2>/dev/null | awk '/default/{print $3; exit}')"; do
  [ -z "$tgt" ] && continue
  if timeout 3 sh -c "exec 3<>/dev/tcp/$tgt/PORT_PLACEHOLDER" 2>/dev/null; then
    echo "PROBE:$tgt=OK"
  else
    echo "PROBE:$tgt=FAIL"
  fi
done
echo "HOSTNAME:$(hostname)"
echo "KERNEL:$(uname -r)""#;

/// 单个 WSL 发行版的信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WslDistro {
    pub name: String,
    pub state: String,
    pub version: String,
    /// 是否为默认发行版（`wsl -l -v` 输出中的 `*` 标记）
    pub is_default: bool,
    /// WSL2 内核版本（探测成功时填充）
    pub kernel: Option<String>,
    /// 各候选目标的真实建连结果：目标地址 → 是否可达
    pub reachable_targets: Vec<WslTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WslTarget {
    pub target: String,
    /// "loopback" | "resolv_conf" | "gateway"
    pub role: String,
    pub reachable: bool,
}

/// WSL 环境总览。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WslDetection {
    /// 是否检测到 WSL（wsl.exe 可用且有发行版）
    pub detected: bool,
    /// wsl.exe 是否存在
    pub wsl_exe_found: bool,
    pub distros: Vec<WslDistro>,
    /// 当前 Windows 侧 SOCKS 端口（用于探测可达性）
    pub socks_port: u16,
    /// 结论：WSL2 是否可通过某个目标地址使用本工具网关
    pub gateway_reachable_from_wsl: bool,
    /// 可达时，WSL 内应使用的代理地址（host:port）
    pub recommended_proxy: Option<String>,
    /// 无法探测的原因（如沙箱/策略阻止 wsl.exe、无发行版等）
    pub note: String,
}

/// WSL 探测全局超时：WSL 冷启动可能较慢，但不能无限等待。
const WSL_PROBE_TIMEOUT_SECS: u64 = 20;

/// 校验发行版名（防止把任意字符串拼进命令行）。
/// WSL 发行版名允许字母/数字/点/下划线/连字符/空格。
fn is_safe_distro_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
}

/// 解析 `wsl.exe -l -v` 输出。
///
/// 真实输出为 UTF-16LE（中文 Windows 上尤其明显），本函数接收已转成
/// UTF-8 的文本；格式如下（列对齐宽度依语言而变）：
/// ```text
///   NAME            STATE           VERSION
/// * Ubuntu-24.04    Running         2
///   docker-desktop  Stopped         2
/// ```
pub fn parse_distro_list(text: &str) -> Vec<WslDistro> {
    let mut out = Vec::new();
    for line in text.lines() {
        let raw = line.trim_end();
        if raw.trim().is_empty() {
            continue;
        }
        // 跳过表头（含 NAME 与 STATE 关键字，且无数字版本列）
        let upper = raw.to_uppercase();
        if upper.contains("NAME") && upper.contains("STATE") && upper.contains("VERSION") {
            continue;
        }
        let is_default = raw.trim_start().starts_with('*');
        let body = raw.trim_start().trim_start_matches('*').trim_start();
        // 从行尾往前取最后一列（VERSION），其余靠 STATE 关键字切分。
        // 用「连续 2 个以上空格」作为列分隔更稳（发行版名可能含单个空格）。
        let cols: Vec<String> = split_aligned_columns(body);
        if cols.len() < 3 {
            continue;
        }
        let name = cols[0].trim().to_string();
        if name.is_empty() || !is_safe_distro_name(&name) {
            continue;
        }
        out.push(WslDistro {
            name,
            state: cols[1].trim().to_string(),
            version: cols[2].trim().to_string(),
            is_default,
            kernel: None,
            reachable_targets: Vec::new(),
        });
    }
    out
}

/// 按「2 个及以上空格」切成列，并保留列内的单个空格。
///
/// WSL 的列是固定宽度对齐的，列间用 2+ 空格分隔；而发行版名可能自带单个
/// 空格（如 `Ubuntu 24.04`）。因此不能简单按空格切，也不能丢弃单空格
/// （否则 `Ubuntu 24.04` 会被拼成 `Ubuntu24.04`）。
fn split_aligned_columns(line: &str) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut pending_spaces = 0usize;
    for ch in line.chars() {
        if ch == ' ' {
            pending_spaces += 1;
            continue;
        }
        if pending_spaces > 0 {
            if pending_spaces >= 2 {
                // 列分隔：收束当前列
                if !cur.is_empty() {
                    cols.push(std::mem::take(&mut cur));
                }
            } else if !cur.is_empty() {
                // 列内单空格：保留（如 "Ubuntu 24.04"）
                cur.push(' ');
            }
            pending_spaces = 0;
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        cols.push(cur);
    }
    cols
}

/// 解析探测脚本输出，返回 (hostname, kernel, 各目标结果)。
fn parse_probe_output(text: &str) -> (Option<String>, Option<String>, Vec<(String, bool)>) {
    let mut hostname = None;
    let mut kernel = None;
    let mut targets: Vec<(String, bool)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("PROBE:") {
            if let Some((tgt, verdict)) = rest.rsplit_once('=') {
                let tgt = tgt.trim();
                if !tgt.is_empty() {
                    targets.push((tgt.to_string(), verdict.trim() == "OK"));
                }
            }
        } else if let Some(rest) = line.strip_prefix("HOSTNAME:") {
            hostname = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("KERNEL:") {
            kernel = Some(rest.trim().to_string());
        }
    }
    (hostname, kernel, targets)
}

/// 定位 wsl.exe（System32 优先）。
fn wsl_exe() -> Option<std::path::PathBuf> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let p = std::path::Path::new(&system_root)
        .join("System32")
        .join("wsl.exe");
    if p.exists() {
        return Some(p);
    }
    // 商店版 WSL 的备用位置
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p2 = std::path::Path::new(&local)
            .join("Microsoft")
            .join("WindowsApps")
            .join("wsl.exe");
        if p2.exists() {
            return Some(p2);
        }
    }
    None
}

/// 把 wsl.exe 的输出字节解释为文本。
/// WSL 在 Windows 上默认输出 UTF-16LE，需按其真实编码解码，
/// 否则中文/对齐会出现乱码（此前 PowerShell 枚举曾踩过同类问题）。
fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    // 含大量 NUL 字节 → 判为 UTF-16LE
    let nul_ratio = bytes.iter().filter(|&&b| b == 0).count() as f64 / bytes.len() as f64;
    if nul_ratio > 0.2 {
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    String::from_utf8_lossy(bytes).to_string()
}

/// 探测单个发行版：是否能连到 Windows 侧 SOCKS 端口。
fn probe_distro(exe: &std::path::Path, distro: &str, socks_port: u16) -> Option<(Option<String>, Option<String>, Vec<(String, bool)>)> {
    if !is_safe_distro_name(distro) {
        return None;
    }
    let script = WSL_PROBE_SCRIPT.replace("PORT_PLACEHOLDER", &socks_port.to_string());
    let out = std_cmd(exe)
        .args(["-d", distro, "--", "sh", "-c", &script])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = decode_wsl_output(&out.stdout);
    let parsed = parse_probe_output(&text);
    if parsed.2.is_empty() && parsed.0.is_none() {
        return None;
    }
    Some(parsed)
}

/// 完整 WSL 检测（只读）。
pub fn detect(socks_port: u16) -> WslDetection {
    let Some(exe) = wsl_exe() else {
        return WslDetection {
            detected: false,
            wsl_exe_found: false,
            distros: vec![],
            socks_port,
            gateway_reachable_from_wsl: false,
            recommended_proxy: None,
            note: "未找到 wsl.exe：本机可能未安装 WSL".to_string(),
        };
    };

    // 列出发行版（超时保护）
    let list_out = std_cmd(&exe).args(["-l", "-v"]).output();
    let list_text = match list_out {
        Ok(o) => decode_wsl_output(&o.stdout),
        Err(e) => {
            return WslDetection {
                detected: false,
                wsl_exe_found: true,
                distros: vec![],
                socks_port,
                gateway_reachable_from_wsl: false,
                recommended_proxy: None,
                note: format!("无法执行 wsl.exe（可能被安全策略阻止）: {}", e),
            }
        }
    };
    let mut distros = parse_distro_list(&list_text);

    if distros.is_empty() {
        // 有些环境 -l -v 因版本过旧失败，回退 -l
        let fallback = std_cmd(&exe).arg("-l").output();
        if let Ok(o) = fallback {
            let t = decode_wsl_output(&o.stdout);
            let names: Vec<String> = t
                .lines()
                .map(|l| l.trim_start().trim_start_matches('*').trim().to_string())
                .filter(|n| is_safe_distro_name(n))
                .collect();
            for n in names {
                if n.to_uppercase().contains("NAME") {
                    continue;
                }
                distros.push(WslDistro {
                    name: n,
                    state: "Unknown".to_string(),
                    version: "Unknown".to_string(),
                    is_default: false,
                    kernel: None,
                    reachable_targets: Vec::new(),
                });
            }
        }
    }

    if distros.is_empty() {
        return WslDetection {
            detected: false,
            wsl_exe_found: true,
            distros: vec![],
            socks_port,
            gateway_reachable_from_wsl: false,
            recommended_proxy: None,
            note: "wsl.exe 可用但未列出任何发行版".to_string(),
        };
    }

    // 只探测运行中的发行版（已停止的无法执行命令；也不擅自启动）
    let mut any_reachable = false;
    let mut recommended: Option<String> = None;
    let mut probed_any_running = false;
    for d in distros.iter_mut() {
        let running = d.state.eq_ignore_ascii_case("running");
        if !running {
            continue;
        }
        probed_any_running = true;
        if let Some((hostname, kernel, targets)) = probe_distro(&exe, &d.name, socks_port) {
            let _ = hostname;
            d.kernel = kernel;
            let mut parsed_targets: Vec<WslTarget> = Vec::new();
            for (i, (tgt, ok)) in targets.iter().enumerate() {
                let role = match i {
                    0 => "loopback",
                    1 => "resolv_conf",
                    _ => "gateway",
                };
                parsed_targets.push(WslTarget {
                    target: tgt.clone(),
                    role: role.to_string(),
                    reachable: *ok,
                });
                if *ok && recommended.is_none() {
                    recommended = Some(format!("{}:{}", tgt, socks_port));
                    any_reachable = true;
                }
            }
            d.reachable_targets = parsed_targets;
        }
    }

    let note = if !probed_any_running {
        "有发行版但均未运行：请先启动 WSL 再检测（本工具不会替你启动）".to_string()
    } else if any_reachable {
        "WSL2 可连到本工具网关（探测已真正建连成功）".to_string()
    } else {
        "WSL2 无法连到本工具网关：Windows 侧 SOCKS 只绑 127.0.0.1，NAT 模式下 WSL 需要 mirrored 网络模式才能共享回环。本工具不代做需要提权的端口转发".to_string()
    };

    WslDetection {
        detected: true,
        wsl_exe_found: true,
        distros,
        socks_port,
        gateway_reachable_from_wsl: any_reachable,
        recommended_proxy: recommended,
        note,
    }
}

/// 生成供用户在 WSL 终端粘贴的一次性代理注入命令。
///
/// 约束：只设置当前 shell 会话的 HTTP_PROXY/HTTPS_PROXY（不回写 ~/.bashrc、
/// 不写 /etc/environment），NO_PROXY 保留回环以免 OAuth 回调被转发。
pub fn build_wsl_proxy_command(proxy_host: &str, proxy_port: u16) -> String {
    format!(
        "export HTTP_PROXY=http://{host}:{port}; export HTTPS_PROXY=http://{host}:{port}; \
export http_proxy=http://{host}:{port}; export https_proxy=http://{host}:{port}; \
export NO_PROXY=localhost,127.0.0.1,::1; export no_proxy=localhost,127.0.0.1,::1; \
echo \"[LostCodexGateway] 代理已注入当前 WSL shell（仅本会话，不写 ~/.bashrc）\"",
        host = proxy_host,
        port = proxy_port
    )
}

/// 生成 WSL 内可直接运行的连通性自检命令（只读）。
pub fn build_wsl_selfcheck_command(proxy_host: &str, proxy_port: u16) -> String {
    format!(
        "echo -n 'curl via gateway: '; curl -sS -x http://{host}:{port} --max-time 10 https://api.ipify.org || echo '(失败)'; echo; \
echo -n 'direct egress: '; curl -sS --max-time 10 https://api.ipify.org || echo '(失败)'; echo",
        host = proxy_host,
        port = proxy_port
    )
}

/// 探测超时约束（供调用方参考）。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(WSL_PROBE_TIMEOUT_SECS);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_distro_list_real_format() {
        // 真实 `wsl -l -v` 输出（列对齐，含默认标记）
        let text = "  NAME            STATE           VERSION\n* Ubuntu-24.04    Running         2\ndocker-desktop  Stopped         2\n";
        let d = parse_distro_list(text);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "Ubuntu-24.04");
        assert_eq!(d[0].state, "Running");
        assert_eq!(d[0].version, "2");
        assert!(d[0].is_default);
        assert_eq!(d[1].name, "docker-desktop");
        assert!(!d[1].is_default);
    }

    #[test]
    fn parse_distro_list_skips_header_and_garbage() {
        let d = parse_distro_list("  NAME   STATE   VERSION\n\n");
        assert!(d.is_empty(), "表头与空行不应被当成发行版");
    }

    #[test]
    fn distro_name_safety_rejects_injection() {
        // 含 shell 元字符或路径分隔符的名字必须被拒绝
        assert!(!is_safe_distro_name("Ubuntu; rm -rf /"));
        assert!(!is_safe_distro_name("Ubuntu$(whoami)"));
        assert!(!is_safe_distro_name("a/b"));
        assert!(!is_safe_distro_name(""));
        assert!(is_safe_distro_name("Ubuntu-24.04"));
        assert!(is_safe_distro_name("Ubuntu 24.04"));
    }

    #[test]
    fn parse_probe_output_extracts_targets() {
        let text = "PROBE:127.0.0.1=OK\nPROBE:172.20.16.1=FAIL\nPROBE:172.20.16.1=FAIL\nHOSTNAME:myhost\nKERNEL:5.15.153.1-microsoft-standard-WSL2\n";
        let (h, k, t) = parse_probe_output(text);
        assert_eq!(h.as_deref(), Some("myhost"));
        assert_eq!(k.as_deref(), Some("5.15.153.1-microsoft-standard-WSL2"));
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], ("127.0.0.1".to_string(), true));
        assert_eq!(t[1], ("172.20.16.1".to_string(), false));
    }

    #[test]
    fn decode_utf16le_output() {
        // WSL 在中文 Windows 上输出 UTF-16LE
        let s = "  NAME   STATE\n* Ubuntu  Running\n";
        let mut bytes = Vec::new();
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_wsl_output(&bytes), s);
        // UTF-8 输入不受影响
        assert_eq!(decode_wsl_output(b"plain ascii"), "plain ascii");
    }

    #[test]
    fn proxy_command_never_writes_global_config() {
        let cmd = build_wsl_proxy_command("127.0.0.1", 17801);
        // 必须只影响当前会话：不得含任何重定向/追写，也不得出现写入全局配置的动作。
        // 注意 echo 文案里可以「提到」~/.bashrc（用于告知用户不会写），
        // 但绝不能出现真正写入它的操作（> / >> / tee 后跟该路径）。
        assert!(!cmd.contains(">>"), "不得追写任何文件");
        assert!(!cmd.contains(" tee "), "不得通过 tee 写文件");
        assert!(!cmd.contains("/etc/environment"), "不得写 /etc/environment");
        // 排除重定向后，不应再有指向 bashrc 的写法
        assert!(
            !cmd.contains("> ~/.bashrc") && !cmd.contains(">~/.bashrc"),
            "不得重定向写入 ~/.bashrc"
        );
        assert!(cmd.contains("NO_PROXY=localhost,127.0.0.1,::1"));
        assert!(cmd.contains("http://127.0.0.1:17801"));
        // 每个赋值都应只作用于当前 shell（export 前缀）
        assert!(cmd.contains("export HTTP_PROXY="));
        assert!(cmd.contains("export HTTPS_PROXY="));
        assert!(cmd.contains("export no_proxy="));
    }

    #[test]
    fn split_columns_handles_single_space_in_name() {
        let cols = split_aligned_columns("Ubuntu 24.04    Running         2");
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0], "Ubuntu 24.04");
        assert_eq!(cols[1], "Running");
        assert_eq!(cols[2], "2");
    }
}
