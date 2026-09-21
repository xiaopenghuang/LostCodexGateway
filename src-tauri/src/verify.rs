//! 出口验证模块：
//! - 本地端口监听探测（仅 127.0.0.1）
//! - 经 SOCKS5（远端 DNS）请求验证端点，回显出口 IP
//! - 只记录域名/状态码/IP/耗时，不记录 URL 完整路径与响应正文

use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyStep {
    pub kind: String, // port_listen | socks_connect | egress_ip | direct_ip
    pub label: String,
    pub ok: bool,
    pub detail: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerifyResult {
    pub ok: bool,
    pub egress_ip: Option<String>,
    pub steps: Vec<VerifyStep>,
    pub started_at: String,
    pub finished_at: String,
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 探测 127.0.0.1:port 是否有 TCP 监听。
pub fn port_listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(2),
    )
    .is_ok()
}

/// 取本机「对照出口」：直接（不经隧道）请求第一个端点，仅用于展示对照。
/// 只尝试一次：对照失败不能拖慢连接流程。
pub async fn direct_egress(endpoints: &[String], timeout_secs: u64) -> VerifyStep {
    if let Some(ep) = endpoints.first() {
        match fetch_ip_direct(ep, timeout_secs).await {
            Ok(ip) => {
                return VerifyStep {
                    kind: "direct_ip".into(),
                    label: "本机对照出口 IP".into(),
                    ok: true,
                    detail: ip,
                    timestamp: now(),
                };
            }
            Err(e) => {
                eprintln!("[verify] direct egress {} failed: {}", ep, e);
            }
        }
    }
    VerifyStep {
        kind: "direct_ip".into(),
        label: "本机对照出口 IP".into(),
        ok: false,
        detail: "直连失败（本机直连被限制或网络策略拦截）".into(),
        timestamp: now(),
    }
}

/// 完整验证序列。返回 VerifyResult。
pub async fn verify_tunnel(
    socks_port: u16,
    endpoints: &[String],
    timeout_secs: u64,
) -> VerifyResult {
    let started_at = now();
    let mut steps: Vec<VerifyStep> = Vec::new();

    // ① 本地端口监听
    let listening = port_listening(socks_port);
    steps.push(VerifyStep {
        kind: "port_listen".into(),
        label: format!("127.0.0.1:{} 端口监听", socks_port),
        ok: listening,
        detail: if listening {
            "监听正常".into()
        } else {
            "端口无监听".into()
        },
        timestamp: now(),
    });

    // ② SOCKS5 握手 + CONNECT（远端 DNS）
    let socks_ok = socks_handshake_test(socks_port).await;
    steps.push(VerifyStep {
        kind: "socks_connect".into(),
        label: "SOCKS5 握手与 CONNECT（远端 DNS）".into(),
        ok: socks_ok,
        detail: if socks_ok {
            "握手成功".into()
        } else {
            "握手失败".into()
        },
        timestamp: now(),
    });

    // ③ 经隧道出口 IP
    let mut egress_ip: Option<String> = None;
    let mut egress_ok = false;
    if socks_ok {
        for ep in endpoints {
            match fetch_ip_via_socks(socks_port, ep, timeout_secs).await {
                Ok(ip) => {
                    egress_ip = Some(ip.clone());
                    egress_ok = true;
                    steps.push(VerifyStep {
                        kind: "egress_ip".into(),
                        label: "隧道出口 IP".into(),
                        ok: true,
                        detail: ip,
                        timestamp: now(),
                    });
                    break;
                }
                Err(e) => {
                    steps.push(VerifyStep {
                        kind: "egress_ip".into(),
                        label: "隧道出口 IP（端点尝试失败）".into(),
                        ok: false,
                        detail: format!("{} 失败: {}", ep, e),
                        timestamp: now(),
                    });
                }
            }
        }
    }

    let ok = listening && socks_ok && egress_ok;
    VerifyResult {
        ok,
        egress_ip,
        steps,
        started_at,
        finished_at: now(),
    }
}

/// 最小 SOCKS5 客户端：无认证握手 + CONNECT（域名形式 = 远端 DNS）。
/// 只做 TCP 层握手与 CONNECT 应答校验，不发送 HTTP 请求。
pub async fn socks_handshake_test(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = tokio::net::TcpStream::connect(addr).await else {
        return false;
    };
    // 无认证握手: 05 01 00
    if stream.write_all(&[0x05, 0x01, 0x00]).await.is_err() {
        return false;
    }
    let mut resp = [0u8; 2];
    if stream.read_exact(&mut resp).await.is_err() {
        return false;
    }
    if resp[0] != 0x05 || resp[1] != 0x00 {
        return false; // 不支持无认证
    }
    // CONNECT api.ipify.org:443（域名形式 = 远端 DNS）
    let host = b"api.ipify.org";
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host);
    req.extend_from_slice(&443u16.to_be_bytes());
    if stream.write_all(&req).await.is_err() {
        return false;
    }
    let mut head = [0u8; 4];
    if stream.read_exact(&mut head).await.is_err() {
        return false;
    }
    if head[0] != 0x05 {
        return false;
    }
    let atyp_len = match head[3] {
        0x01 => 4 + 2,
        0x03 => {
            let mut len = [0u8; 1];
            if stream.read_exact(&mut len).await.is_err() {
                return false;
            }
            len[0] as usize + 2
        }
        0x04 => 16 + 2,
        _ => return false,
    };
    let mut rest = vec![0u8; atyp_len];
    let _ = stream.read_exact(&mut rest).await;
    head[1] == 0x00 // REP == succeeded
}

/// IP 协议版本。
pub fn ip_version(ip: &str) -> &'static str {
    if is_ipv4(ip) {
        "IPv4"
    } else if is_ipv6(ip) {
        "IPv6"
    } else {
        "未知"
    }
}

/// 经 SOCKS（远端 DNS）探测目标 host:port 的 TCP 连通性；返回耗时（毫秒）。
/// 与握手不同：这一步证明「转发 + 远端建连」真实可用。
pub async fn socks_tcp_probe(
    socks_port: u16,
    host: &str,
    dst_port: u16,
    timeout_secs: u64,
) -> Result<u64, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let started = std::time::Instant::now();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], socks_port));
    let mut stream = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .map_err(|_| "连 SOCKS 端口超时".to_string())?
    .map_err(|e| e.to_string())?;
    // 无认证握手
    stream.write_all(&[0x05, 0x01, 0x00]).await.map_err(|e| e.to_string())?;
    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await.map_err(|e| e.to_string())?;
    if resp != [0x05, 0x00] {
        return Err("SOCKS5 握手被拒绝".to_string());
    }
    // CONNECT（域名形式 = 远端 DNS）
    let hb = host.as_bytes();
    if hb.len() > 255 {
        return Err("域名过长".to_string());
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, hb.len() as u8];
    req.extend_from_slice(hb);
    req.extend_from_slice(&dst_port.to_be_bytes());
    stream.write_all(&req).await.map_err(|e| e.to_string())?;
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(|e| e.to_string())?;
    if head[0] != 0x05 {
        return Err("SOCKS5 应答无效".to_string());
    }
    let atyp_len = match head[3] {
        0x01 => 6,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(|e| e.to_string())?;
            len[0] as usize + 2
        }
        0x04 => 18,
        _ => return Err("SOCKS5 应答 ATYP 无效".to_string()),
    };
    let mut rest = vec![0u8; atyp_len];
    stream.read_exact(&mut rest).await.map_err(|e| e.to_string())?;
    if head[1] != 0x00 {
        return Err(format!("远端拒绝建连 (REP={})", head[1]));
    }
    Ok(started.elapsed().as_millis() as u64)
}

/// 出口探测结果：IP + 版本 + 耗时。
#[derive(Debug, Clone)]
pub struct EgressProbe {
    pub ip: String,
    pub version: String,
    pub latency_ms: u64,
    pub source: String, // 使用的端点域名
}

/// 经 SOCKS 依次尝试端点，返回第一个成功（IP+版本+耗时）。
pub async fn probe_egress_via_socks(
    socks_port: u16,
    endpoints: &[String],
    timeout_secs: u64,
) -> Option<EgressProbe> {
    for ep in endpoints {
        let started = std::time::Instant::now();
        if let Ok(ip) = fetch_ip_via_socks(socks_port, ep, timeout_secs).await {
            return Some(EgressProbe {
                version: ip_version(&ip).to_string(),
                ip,
                latency_ms: started.elapsed().as_millis() as u64,
                source: ep.clone(),
            });
        }
    }
    None
}

/// 直连依次尝试端点，返回第一个成功（IP+版本+耗时）。
pub async fn probe_egress_direct(
    endpoints: &[String],
    timeout_secs: u64,
) -> Option<EgressProbe> {
    for ep in endpoints {
        let started = std::time::Instant::now();
        if let Ok(ip) = fetch_ip_direct(ep, timeout_secs).await {
            return Some(EgressProbe {
                version: ip_version(&ip).to_string(),
                ip,
                latency_ms: started.elapsed().as_millis() as u64,
                source: ep.clone(),
            });
        }
    }
    None
}

/// 经 SOCKS 代理 GET 任意 URL，返回 (状态码, 内容首行)。
/// 只用于隧道可达性验证（如内部测试服务），不解析 IP、不记录正文。
pub async fn fetch_http_via_socks(
    port: u16,
    url: &str,
    timeout_secs: u64,
) -> Result<(u16, String), String> {
    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", port))
        .map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let first = body.lines().next().unwrap_or("").to_string();
    Ok((status, first))
}

/// 经 SOCKS 代理 GET 端点并解析出口 IP。
async fn fetch_ip_via_socks(port: u16, endpoint: &str, timeout_secs: u64) -> Result<String, String> {
    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", port))
        .map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(endpoint)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    parse_ip_response(&body).ok_or_else(|| "响应中未找到 IP".to_string())
}

/// 直连（不经隧道）GET 端点解析出口 IP。
async fn fetch_ip_direct(endpoint: &str, timeout_secs: u64) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(endpoint)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    parse_ip_response(&body).ok_or_else(|| "响应中未找到 IP".to_string())
}

/// 从常见 IP 端点响应中提取 IP 字符串（JSON {"ip":"x"} 或纯文本）。
fn parse_ip_response(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if is_ipv4(trimmed) || is_ipv6(trimmed) {
        return Some(trimmed.to_string());
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        for key in ["ip", "query", "client_ip"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                if is_ipv4(s) || is_ipv6(s) {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

pub fn is_ipv4(s: &str) -> bool {
    s.parse::<std::net::Ipv4Addr>().is_ok()
}
pub fn is_ipv6(s: &str) -> bool {
    s.parse::<std::net::Ipv6Addr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ip_plain() {
        assert_eq!(parse_ip_response("1.2.3.4\n"), Some("1.2.3.4".to_string()));
        assert_eq!(
            parse_ip_response("2606:4700::1111"),
            Some("2606:4700::1111".to_string())
        );
    }

    #[test]
    fn parse_ip_json() {
        assert_eq!(
            parse_ip_response(r#"{"ip":"203.0.113.7"}"#),
            Some("203.0.113.7".to_string())
        );
        assert_eq!(parse_ip_response(r#"{"hello":"world"}"#), None);
        assert_eq!(parse_ip_response("not an ip"), None);
    }

    #[test]
    fn port_listening_negative() {
        assert!(!port_listening(65530));
    }
}
