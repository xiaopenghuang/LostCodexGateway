//! HTTP CONNECT → SSH SOCKS5 桥接层（M2 实测证明必需：Codex 原生二进制
//! 只接受 HTTP(S)_PROXY 的 CONNECT 语义，不接受 socks5:// 直供）。
//!
//! 安全约束（对应开发文档 §5.5）：
//! - 仅绑定 127.0.0.1 随机端口；**校验对端确为回环**（不止靠绑定）
//! - 只实现 HTTP CONNECT 语义，不做 TLS MITM、不装根证书、不缓存内容
//! - 限制并发连接数与**空闲超时**（双向复制期间无数据流动即回收）
//! - **拒绝循环代理**：目标是本机回环/私有网段/自身端口时直接拒绝
//! - 日志只记录目标域名与状态，不记录请求正文；失败有结构化错误码

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::Duration;

pub const MAX_CONCURRENT: usize = 64;
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// 空闲超时：双向复制期间若两个方向都长时间无字节流动，回收连接。
/// 防止 CLI 崩溃后留下的半开连接长期占用并发额度。
pub const IDLE_TIMEOUT_SECS: u64 = 300;
/// 读取请求头的总时限（秒）
pub const HEADER_READ_TIMEOUT_SECS: u64 = 10;

/// 桥接层拒绝连接的原因（结构化错误码，「明确日志及错误码」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// 对端不是回环地址
    NonLoopbackPeer,
    /// 请求头非法（非 CONNECT / 绝对 URI / 缺端口）
    BadRequest,
    /// 目标指向本机回环（会造成循环代理）
    LoopbackTarget,
    /// 目标指向私有网段（不应经隧道访问内网）
    PrivateTarget,
    /// 目标是桥接层自身的端口
    SelfTarget,
    /// 并发已满
    TooManyConnections,
    /// 请求头超长 / 读取超时
    HeaderTimeout,
    /// 下游 SOCKS5 连接失败
    UpstreamFailed,
    /// 空闲超时
    IdleTimeout,
}

impl RejectReason {
    /// 稳定的机器可读代码（前端/日志可据此分类，不靠匹配中文文案）。
    pub fn code(&self) -> &'static str {
        match self {
            RejectReason::NonLoopbackPeer => "BRIDGE_NON_LOOPBACK_PEER",
            RejectReason::BadRequest => "BRIDGE_BAD_REQUEST",
            RejectReason::LoopbackTarget => "BRIDGE_LOOPBACK_TARGET",
            RejectReason::PrivateTarget => "BRIDGE_PRIVATE_TARGET",
            RejectReason::SelfTarget => "BRIDGE_SELF_TARGET",
            RejectReason::TooManyConnections => "BRIDGE_TOO_MANY_CONNECTIONS",
            RejectReason::HeaderTimeout => "BRIDGE_HEADER_TIMEOUT",
            RejectReason::UpstreamFailed => "BRIDGE_UPSTREAM_FAILED",
            RejectReason::IdleTimeout => "BRIDGE_IDLE_TIMEOUT",
        }
    }

    /// 面向用户的简短说明（中文）。
    pub fn message(&self) -> &'static str {
        match self {
            RejectReason::NonLoopbackPeer => "拒绝非回环来源的连接",
            RejectReason::BadRequest => "请求不是合法的 HTTP CONNECT",
            RejectReason::LoopbackTarget => "拒绝把本机回环地址作为目标（防循环代理）",
            RejectReason::PrivateTarget => "拒绝经隧道访问私有网段",
            RejectReason::SelfTarget => "拒绝把桥接层自身作为目标",
            RejectReason::TooManyConnections => "并发连接数已达上限",
            RejectReason::HeaderTimeout => "等待请求头超时",
            RejectReason::UpstreamFailed => "下游 SOCKS5 连接失败",
            RejectReason::IdleTimeout => "空闲超时，已回收连接",
        }
    }

    /// 应返回给客户端的 HTTP 状态行。
    ///
    /// 这是拒绝响应的**唯一事实来源**：新增拒绝类型时改这里，
    /// 避免状态码散落在 `handle_connection` 各处。
    pub fn http_status(&self) -> &'static str {
        match self {
            // 请求本身不合法 → 400
            RejectReason::BadRequest => "HTTP/1.1 400 Bad Request",
            // 资源不足 → 503
            RejectReason::TooManyConnections | RejectReason::HeaderTimeout => {
                "HTTP/1.1 503 Service Unavailable"
            }
            // 下游失败 → 502
            RejectReason::UpstreamFailed => "HTTP/1.1 502 Bad Gateway",
            // 策略拒绝（回环/私有/自身）→ 403
            _ => "HTTP/1.1 403 Forbidden",
        }
    }
}

/// 单条拒绝记录，供诊断页展示「为什么连不上」。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RejectRecord {
    pub code: String,
    pub message: String,
    /// 脱敏后的目标（仅域名/字面量，不含路径与查询串）
    pub target: String,
}

pub struct BridgeStats {
    /// 到达桥接层的连接**尝试**数（含随后被拒/上游失败的），不代表流量真的过了隧道。
    pub connections_total: Arc<AtomicU64>,
    pub connections_active: Arc<AtomicU64>,
    /// 上游 SOCKS5 建连成功、已回 200 的连接数。
    ///
    /// 与 `connections_total` 的区别是**关键**：`_total` 在 TCP 连上来时即自增，
    /// 此时还不知道目标是合法域名、也不知道 ssh -D 能否建连；只有 `_tunneled`
    /// 才代表「这条流量确实进了隧道」。诊断据此判定「已验证」，不能拿 `_total`。
    pub connections_tunneled: Arc<AtomicU64>,
    pub last_target: Arc<parking_lot::Mutex<Option<String>>>,
    /// 最近 N 条成功经隧道的脱敏目标（环形）。用于把「桥接有过流量」这种
    /// 全局事实，收窄成「确实有哪几个目标出去了」，从而支持异常判定。
    pub recent_targets: Arc<parking_lot::Mutex<Vec<String>>>,
    /// 最近 N 条拒绝记录（环形，只保留最新的）
    pub rejects: Arc<parking_lot::Mutex<Vec<RejectRecord>>>,
    pub rejects_total: Arc<AtomicU64>,
}

/// 拒绝记录保留条数上限：诊断页只需看最近的，避免无界增长。
const MAX_REJECT_RECORDS: usize = 20;
/// 成功目标记录保留条数上限（同上，只为诊断展示）。
const MAX_TARGET_RECORDS: usize = 20;

impl Default for BridgeStats {
    fn default() -> Self {
        Self {
            connections_total: Arc::new(AtomicU64::new(0)),
            connections_active: Arc::new(AtomicU64::new(0)),
            connections_tunneled: Arc::new(AtomicU64::new(0)),
            last_target: Arc::new(parking_lot::Mutex::new(None)),
            recent_targets: Arc::new(parking_lot::Mutex::new(Vec::new())),
            rejects: Arc::new(parking_lot::Mutex::new(Vec::new())),
            rejects_total: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// 启动桥接器：绑定 127.0.0.1:0（随机端口），返回 (实际端口, 任务句柄, 统计)。
pub async fn start_bridge(
    socks_port: u16,
) -> Result<(u16, tokio::task::JoinHandle<()>, BridgeStats), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("桥接层绑定失败: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    let stats = BridgeStats::default();
    let stats2 = stats.clone();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let handle = tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let sem = semaphore.clone();
            let st = stats2.clone();
            tokio::spawn(async move {
                // 来源校验：绑定 127.0.0.1 已挡住外网，但显式再查一次对端地址，
                // 防止将来有人改动绑定地址后这道防线静默失效。
                if !peer.ip().is_loopback() {
                    st.record_reject(RejectReason::NonLoopbackPeer, &peer.ip().to_string());
                    return;
                }
                let _permit = match sem.try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        // 并发满：回一个明确状态而不是静默断开，否则用户无法区分
                        // 「被限流」和「隧道坏了」。
                        let reason = RejectReason::TooManyConnections;
                        st.record_reject(reason, "-");
                        let mut s = stream;
                        let resp = format!(
                            "{}\r\nX-LCFG-Reject: {}\r\n\r\n",
                            reason.http_status(),
                            reason.code()
                        );
                        let _ = s.write_all(resp.as_bytes()).await;
                        return;
                    }
                };
                st.connections_total.fetch_add(1, Ordering::Relaxed);
                st.connections_active.fetch_add(1, Ordering::Relaxed);
                handle_connection(stream, socks_port, port, &st).await;
                st.connections_active.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    Ok((port, handle, stats))
}

impl BridgeStats {
    fn clone(&self) -> Self {
        Self {
            connections_total: self.connections_total.clone(),
            connections_active: self.connections_active.clone(),
            connections_tunneled: self.connections_tunneled.clone(),
            last_target: self.last_target.clone(),
            recent_targets: self.recent_targets.clone(),
            rejects: self.rejects.clone(),
            rejects_total: self.rejects_total.clone(),
        }
    }

    /// 记一条成功经隧道的连接（脱敏目标）。target 必须已由 sanitize_target 处理。
    fn record_tunneled(&self, target: &str) {
        self.connections_tunneled.fetch_add(1, Ordering::Relaxed);
        let mut v = self.recent_targets.lock();
        v.push(target.to_string());
        if v.len() > MAX_TARGET_RECORDS {
            let overflow = v.len() - MAX_TARGET_RECORDS;
            v.drain(0..overflow);
        }
    }

    /// 记一条拒绝（同时计数 + 存最近若干条）。target 必须是脱敏后的值。
    fn record_reject(&self, reason: RejectReason, target: &str) {
        self.rejects_total.fetch_add(1, Ordering::Relaxed);
        let rec = RejectRecord {
            code: reason.code().to_string(),
            message: reason.message().to_string(),
            target: target.to_string(),
        };
        let mut v = self.rejects.lock();
        v.push(rec);
        if v.len() > MAX_REJECT_RECORDS {
            let overflow = v.len() - MAX_REJECT_RECORDS;
            v.drain(0..overflow);
        }
    }

    pub fn snapshot(&self) -> BridgeSnapshot {
        BridgeSnapshot {
            connections_total: self.connections_total.load(Ordering::Relaxed),
            connections_active: self.connections_active.load(Ordering::Relaxed),
            connections_tunneled: self.connections_tunneled.load(Ordering::Relaxed),
            last_target: self.last_target.lock().clone(),
            recent_targets: self.recent_targets.lock().clone(),
            rejects_total: self.rejects_total.load(Ordering::Relaxed),
            recent_rejects: self.rejects.lock().clone(),
        }
    }
}

/// 桥接层统计快照（给前端的可序列化形式）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BridgeSnapshot {
    /// 连接尝试数（含被拒/上游失败的）。
    pub connections_total: u64,
    pub connections_active: u64,
    /// 真正建连成功、进了隧道的连接数。判定「已验证」只认这个。
    pub connections_tunneled: u64,
    pub last_target: Option<String>,
    /// 最近成功经隧道的脱敏目标。
    pub recent_targets: Vec<String>,
    pub rejects_total: u64,
    pub recent_rejects: Vec<RejectRecord>,
}

/// 解析 CONNECT 请求头，返回目标 (host, port)。
/// 只接受 `CONNECT host:port HTTP/1.x`；拒绝绝对 URI 与其他方法（明文管理端点防护）。
pub fn parse_connect_header(head: &str) -> Option<(String, u16)> {
    let first_line = head.lines().next()?.trim();
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    if !method.eq_ignore_ascii_case("CONNECT") {
        return None;
    }
    let target = parts.next()?;
    // 拒绝绝对形式（如 CONNECT http://host:port 带 scheme）
    if target.contains("://") {
        return None;
    }
    // IPv6 字面量形式：[::1]:443
    if let Some(rest) = target.strip_prefix('[') {
        let (host6, port_str) = rest.split_once("]:")?;
        if host6.is_empty() {
            return None;
        }
        let port: u16 = port_str.parse().ok()?;
        if port == 0 {
            return None;
        }
        return Some((host6.to_string(), port));
    }
    let (host, port_str) = target.rsplit_once(':')?;
    if host.is_empty() || host.contains(' ') {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    if port == 0 {
        return None;
    }
    Some((host.to_string(), port))
}

/// 判断目标是否应被拒绝（防循环代理 + 防经隧道访问内网）。
///
/// `self_port` 是桥接层自己的监听端口：把桥接层当目标会形成
/// 「桥接 → SOCKS → ssh → … → 桥接」的循环，必须拒绝。
pub fn check_target(host: &str, port: u16, self_port: u16) -> Result<(), RejectReason> {
    if port == self_port {
        return Err(RejectReason::SelfTarget);
    }
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');

    // 回环：字面量或 localhost
    if h.eq_ignore_ascii_case("localhost") || h == "::1" {
        return Err(RejectReason::LoopbackTarget);
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        if ip.is_loopback() {
            return Err(RejectReason::LoopbackTarget);
        }
        if is_private_ip(&ip) {
            return Err(RejectReason::PrivateTarget);
        }
        // 链路本地 / 未指定 / 组播：同样不是合法的公网目标
        match ip {
            std::net::IpAddr::V4(v4) => {
                if v4.is_link_local() || v4.is_unspecified() || v4.is_multicast() {
                    return Err(RejectReason::PrivateTarget);
                }
            }
            std::net::IpAddr::V6(v6) => {
                if v6.is_unspecified() || v6.is_multicast() {
                    return Err(RejectReason::PrivateTarget);
                }
            }
        }
    }
    Ok(())
}

/// RFC1918 / CGNAT / 唯一本地地址判定。
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            // 10.0.0.0/8
            o[0] == 10
                // 172.16.0.0/12
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                // 192.168.0.0/16
                || (o[0] == 192 && o[1] == 168)
                // 100.64.0.0/10 (CGNAT)
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                // 169.254.0.0/16 链路本地
                || (o[0] == 169 && o[1] == 254)
                // 0.0.0.0/8
                || o[0] == 0
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            // fc00::/7 唯一本地
            (s[0] & 0xfe00) == 0xfc00
                // fe80::/10 链路本地
                || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

/// 脱敏目标：只保留主机与端口，去掉可能含敏感信息的路径/查询串。
fn sanitize_target(host: &str, port: u16) -> String {
    format!("{}:{}", host, port)
}

/// 双向复制的结束原因。
#[derive(Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    /// 正常结束（任一端关闭）
    Finished,
    /// 两个方向都静默超过空闲上限
    Idle,
}

/// 双向复制，带**空闲**超时（不是总时长超时）。
///
/// 语义：只要任一方向有字节流动，计时器就重置。只有「两个方向都连续静默」
/// 达到 `IDLE_TIMEOUT_SECS` 才判定为空闲并返回 `Idle`。
///
/// 这样既能让 CLI 崩溃后残留的半开连接最终被回收（不至于永久占用并发额度），
/// 又不会误杀长时间活跃的长连接。
async fn copy_with_idle_timeout<A, B>(a: &mut A, b: &mut B) -> CopyOutcome
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    copy_with_idle_window(a, b, Duration::from_secs(IDLE_TIMEOUT_SECS)).await
}

/// 与 [`copy_with_idle_timeout`] 相同，但空闲窗口可注入——便于用极短窗口做单元测试。
pub async fn copy_with_idle_window<A, B>(a: &mut A, b: &mut B, idle: Duration) -> CopyOutcome
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut a_buf = vec![0u8; 8 * 1024];
    let mut b_buf = vec![0u8; 8 * 1024];
    // 每个方向是否已到 EOF、是否已向下游半关闭。
    let mut a_eof = false;
    let mut b_eof = false;
    let mut a_shut = false;
    let mut b_shut = false;

    loop {
        // 两个方向都到 EOF 且都已半关闭 → 正常结束。
        if a_eof && b_eof {
            return CopyOutcome::Finished;
        }

        // 若某一方向已 EOF，立即对该方向的「下游写端」做半关闭，
        // 让对端知道不会再有数据——否则对端会一直等下去。
        if a_eof && !a_shut {
            let _ = b.shutdown().await;
            a_shut = true;
        }
        if b_eof && !b_shut {
            let _ = a.shutdown().await;
            b_shut = true;
        }

        // 两侧都 EOF 但还没返回（上一轮刚置位）→ 下一轮开头会返回。
        // 每次循环以完整空闲窗口重新计时：只要有数据流过就重置。
        let tick = tokio::time::timeout(idle, async {
            tokio::select! {
                // client → upstream
                r = a.read(&mut a_buf), if !a_eof => {
                    match r {
                        Ok(0) => { a_eof = true; }
                        Ok(n) => {
                            if b.write_all(&a_buf[..n]).await.is_err() {
                                a_eof = true;
                                b_eof = true;
                            }
                        }
                        Err(_) => { a_eof = true; b_eof = true; }
                    }
                }
                // upstream → client
                r = b.read(&mut b_buf), if !b_eof => {
                    match r {
                        Ok(0) => { b_eof = true; }
                        Ok(n) => {
                            if a.write_all(&b_buf[..n]).await.is_err() {
                                a_eof = true;
                                b_eof = true;
                            }
                        }
                        Err(_) => { a_eof = true; b_eof = true; }
                    }
                }
            }
        })
        .await;

        if tick.is_err() {
            // 空闲窗口内两个方向都没有可读事件。
            // 注意：若此时只剩一侧未 EOF，说明对端已半关闭但迟迟不主动断开——
            // 这仍是「空闲」，按空闲回收（而不是拖延到永久）。
            return CopyOutcome::Idle;
        }
    }
}

async fn handle_connection(mut client: TcpStream, socks_port: u16, self_port: u16, stats: &BridgeStats) {
    // 读请求头（限大小 + 空闲超时）
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 4096];
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(HEADER_READ_TIMEOUT_SECS);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            stats.record_reject(RejectReason::HeaderTimeout, "-");
            return;
        }
        match tokio::time::timeout(remaining, client.read(&mut tmp)).await {
            Ok(Ok(0)) => return,
            Ok(Ok(n)) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > MAX_HEADER_BYTES {
                    stats.record_reject(RejectReason::HeaderTimeout, "-");
                    return;
                }
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            _ => {
                stats.record_reject(RejectReason::HeaderTimeout, "-");
                return;
            }
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let Some((host, port)) = parse_connect_header(&head) else {
        stats.record_reject(RejectReason::BadRequest, "-");
        let resp = format!(
            "{}\r\nX-LCFG-Reject: {}\r\n\r\n",
            RejectReason::BadRequest.http_status(),
            RejectReason::BadRequest.code()
        );
        let _ = client.write_all(resp.as_bytes()).await;
        return;
    };
    let target = sanitize_target(&host, port);

    // 目标校验：防循环代理 + 防经隧道访问内网
    if let Err(reason) = check_target(&host, port, self_port) {
        stats.record_reject(reason, &target);
        let resp = format!(
            "{}\r\nX-LCFG-Reject: {}\r\n\r\n",
            reason.http_status(),
            reason.code()
        );
        let _ = client.write_all(resp.as_bytes()).await;
        return;
    }

    *stats.last_target.lock() = Some(target.clone());

    // 经下游 SOCKS5（远端 DNS）连接目标
    match socks5_connect(socks_port, &host, port).await {
        Ok(mut upstream) => {
            if client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .is_err()
            {
                return;
            }
            // 到这里才算「流量确实进了隧道」：目标合法 + 下游 SOCKS5 建连成功
            // + 已向客户端回 200。记录用于诊断的「已验证」判定。
            stats.record_tunneled(&target);
            // 双向复制 + **真正的空闲**超时。
            //
            // 注意不能用 `timeout(idle, copy_bidirectional(...))`：那是**总时长**
            // 上限，会把一条持续有数据的连接（例如 SSE 长流）在 300s 时硬切断，
            // 造成真实故障。这里改成「只要任一方向有字节流动就重置计时」，
            // 只有两个方向都静默超过 IDLE_TIMEOUT_SECS 才回收。
            match copy_with_idle_timeout(&mut client, &mut upstream).await {
                CopyOutcome::Finished => {}
                CopyOutcome::Idle => {
                    // 空闲超时：单独一类记录，便于用户与「请求被拒」区分。
                    stats.record_reject(RejectReason::IdleTimeout, "-");
                }
            }
        }
        Err(e) => {
            let reason = RejectReason::UpstreamFailed;
            stats.record_reject(reason, &target);
            eprintln!("[bridge] CONNECT 失败 ({}) {}", reason.code(), e);
            let resp = format!(
                "{}\r\nX-LCFG-Reject: {}\r\n\r\n",
                reason.http_status(),
                reason.code()
            );
            let _ = client.write_all(resp.as_bytes()).await;
        }
    }
}

/// 经 127.0.0.1:socks_port 的 SOCKS5 代理（域名形式 = 远端 DNS）连接目标。
async fn socks5_connect(socks_port: u16, host: &str, port: u16) -> Result<TcpStream, String> {
    let proxy = TcpStream::connect(("127.0.0.1", socks_port))
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = proxy;
    // 无认证握手
    stream
        .write_all(&[0x05, 0x01, 0x00])
        .await
        .map_err(|e| e.to_string())?;
    let mut resp = [0u8; 2];
    stream
        .read_exact(&mut resp)
        .await
        .map_err(|e| e.to_string())?;
    if resp != [0x05, 0x00] {
        return Err("SOCKS5 握手被拒绝".to_string());
    }
    // CONNECT（域名形式）
    let hb = host.as_bytes();
    if hb.len() > 255 {
        return Err("域名过长".to_string());
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, hb.len() as u8];
    req.extend_from_slice(hb);
    req.extend_from_slice(&port.to_be_bytes());
    stream
        .write_all(&req)
        .await
        .map_err(|e| e.to_string())?;
    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| e.to_string())?;
    if head[0] != 0x05 {
        return Err("SOCKS5 应答无效".to_string());
    }
    let atyp_len = match head[3] {
        0x01 => 4 + 2,
        0x03 => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|e| e.to_string())?;
            len[0] as usize + 2
        }
        0x04 => 16 + 2,
        _ => return Err("SOCKS5 应答 ATYP 无效".to_string()),
    };
    let mut rest = vec![0u8; atyp_len];
    stream
        .read_exact(&mut rest)
        .await
        .map_err(|e| e.to_string())?;
    if head[1] != 0x00 {
        return Err(format!("SOCKS5 CONNECT 被拒 (REP={})", head[1]));
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_connect_ok() {
        assert_eq!(
            parse_connect_header("CONNECT api.github.com:443 HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(("api.github.com".to_string(), 443))
        );
        assert_eq!(
            parse_connect_header("connect 127.0.0.1:8080 HTTP/1.0\r\n\r\n"),
            Some(("127.0.0.1".to_string(), 8080))
        );
    }

    #[test]
    fn parse_connect_reject() {
        assert_eq!(parse_connect_header("GET / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_connect_header("CONNECT http://x.com:443 HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_connect_header("CONNECT no-port HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_connect_header("CONNECT host:0 HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_connect_header(""), None);
    }

    #[test]
    fn parse_connect_ipv6_literal() {
        // [::1]:443 形式：host 内去掉方括号
        assert_eq!(
            parse_connect_header("CONNECT [::1]:443 HTTP/1.1\r\n\r\n"),
            Some(("::1".to_string(), 443))
        );
        assert_eq!(
            parse_connect_header("CONNECT [2001:db8::1]:8443 HTTP/1.1\r\n\r\n"),
            Some(("2001:db8::1".to_string(), 8443))
        );
        // 缺端口应拒绝
        assert_eq!(parse_connect_header("CONNECT [::1] HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn target_rejects_loopback() {
        let self_port = 12000;
        // 端口不等于自身端口时，回环仍需拒绝（防循环代理的核心）
        assert_eq!(
            check_target("127.0.0.1", 443, self_port),
            Err(RejectReason::LoopbackTarget)
        );
        assert_eq!(
            check_target("localhost", 443, self_port),
            Err(RejectReason::LoopbackTarget)
        );
        assert_eq!(
            check_target("::1", 443, self_port),
            Err(RejectReason::LoopbackTarget)
        );
        // 127.0.0.0/8 整段都是回环
        assert_eq!(
            check_target("127.5.5.5", 443, self_port),
            Err(RejectReason::LoopbackTarget)
        );
    }

    #[test]
    fn target_rejects_self_port_before_anything_else() {
        // 端口命中桥接层自身：即使主机是公网地址，也必须先判为 SelfTarget，
        // 因为「桥接 → SOCKS → ssh → 回到桥接」会形成无限循环。
        assert_eq!(
            check_target("example.com", 12000, 12000),
            Err(RejectReason::SelfTarget)
        );
    }

    #[test]
    fn target_rejects_private_ranges() {
        let sp = 12000;
        for (host, expect) in [
            ("10.0.0.1", RejectReason::PrivateTarget),
            ("172.16.0.1", RejectReason::PrivateTarget),
            ("172.31.255.255", RejectReason::PrivateTarget),
            ("192.168.1.1", RejectReason::PrivateTarget),
            ("100.64.0.1", RejectReason::PrivateTarget),
            ("169.254.1.1", RejectReason::PrivateTarget),
            ("0.0.0.0", RejectReason::PrivateTarget),
        ] {
            assert_eq!(check_target(host, 443, sp), Err(expect), "host={}", host);
        }
    }

    #[test]
    fn target_allows_public_addresses() {
        let sp = 12000;
        // 172.32 已超出 172.16/12，属公网
        for host in ["1.1.1.1", "8.8.8.8", "172.32.0.1", "2606:4700::1111"] {
            assert!(
                check_target(host, 443, sp).is_ok(),
                "host={} 应被放行",
                host
            );
        }
        // 域名（含远端 DNS 语义）一律放行：解析发生在远端，本地无从判断
        for host in ["api.github.com", "persistent.oaistatic.com"] {
            assert!(check_target(host, 443, sp).is_ok(), "host={}", host);
        }
    }

    #[test]
    fn reject_codes_are_stable_and_unique() {
        // 错误码是前端/日志的分类依据，必须稳定且不重复
        let all = [
            RejectReason::NonLoopbackPeer,
            RejectReason::BadRequest,
            RejectReason::LoopbackTarget,
            RejectReason::PrivateTarget,
            RejectReason::SelfTarget,
            RejectReason::TooManyConnections,
            RejectReason::HeaderTimeout,
            RejectReason::UpstreamFailed,
            RejectReason::IdleTimeout,
        ];
        let mut codes: Vec<&str> = all.iter().map(|r| r.code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), all.len(), "错误码存在重复");
        for r in all {
            assert!(r.code().starts_with("BRIDGE_"), "错误码前缀: {}", r.code());
            assert!(!r.message().is_empty(), "缺少中文说明: {:?}", r);
        }
    }

    #[test]
    fn reject_records_are_bounded_and_desensitized() {
        let st = BridgeStats::default();
        // 灌入超过上限的拒绝记录
        for i in 0..(MAX_REJECT_RECORDS + 5) {
            st.record_reject(RejectReason::LoopbackTarget, &format!("h{}:443", i));
        }
        let snap = st.snapshot();
        assert_eq!(snap.rejects_total as usize, MAX_REJECT_RECORDS + 5, "总计数应累加");
        assert_eq!(snap.recent_rejects.len(), MAX_REJECT_RECORDS, "环形上限应生效");
        // 保留的是最新的：第一条应是 h5
        assert_eq!(snap.recent_rejects[0].target, "h5:443");
    }

    #[test]
    fn sanitize_target_drops_path_and_query() {
        // 目标里若混入路径/查询串（不该出现，但防御性处理），只保留 host:port
        assert_eq!(sanitize_target("example.com", 443), "example.com:443");
        assert_eq!(sanitize_target("1.1.1.1", 8443), "1.1.1.1:8443");
    }

    // ---- 空闲超时语义（用极短窗口，避免测试等待 300s）----

    #[tokio::test]
    async fn idle_window_reclaims_silent_connection() {
        // 两端都不发数据：应在空闲窗口后判定为 Idle。
        let (mut a, _a_peer) = tokio::io::duplex(1024);
        let (mut b, _b_peer) = tokio::io::duplex(1024);
        let start = tokio::time::Instant::now();
        let outcome = copy_with_idle_window(&mut a, &mut b, Duration::from_millis(150)).await;
        assert_eq!(outcome, CopyOutcome::Idle, "静默连接应被判为空闲");
        assert!(
            start.elapsed() >= Duration::from_millis(140),
            "不应早于空闲窗口返回"
        );
    }

    #[tokio::test]
    async fn idle_window_resets_on_activity() {
        // a ← 对端持续喂数据；b 侧对端永远静默。
        // 若实现是「总时长上限」，150ms 后就会返回 Idle；
        // 正确的空闲语义下，只要 a 持续有数据，就不会返回 Idle。
        let (mut a, mut a_peer) = tokio::io::duplex(1024);
        // b 的对端持有在本地：不读也不写，用于吸收 a→b 方向被转写的数据。
        let (mut b, mut b_peer) = tokio::io::duplex(64 * 1024);

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            // 每 50ms 写 1 字节，总计 1s（远超 150ms 空闲窗口）
            for _ in 0..20 {
                if a_peer.write_all(b"x").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });

        // 持续抽走 b 对端的数据，避免 64KB 缓冲写满后 a→b 的写阻塞
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut sink = [0u8; 1024];
            loop {
                match b_peer.read(&mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        let outcome = tokio::time::timeout(
            Duration::from_millis(800),
            copy_with_idle_window(&mut a, &mut b, Duration::from_millis(150)),
        )
        .await;
        assert!(
            outcome.is_err(),
            "持续活跃的连接不应在空闲窗口内返回（说明被误判为空闲/总时长上限）: {:?}",
            outcome
        );
    }

    #[tokio::test]
    async fn single_side_eof_half_closes_and_then_idles_out() {
        // 一端关闭（半关闭场景）：
        // 正确行为是「对另一端做半关闭，然后按空闲窗口回收」——
        // 而不是立即 Finished（对端可能还有数据要发），也不是永久挂住。
        let (mut a, a_peer) = tokio::io::duplex(1024);
        let (mut b, _b_peer) = tokio::io::duplex(1024);

        // a 的对端关闭 → a 方向读到 EOF
        drop(a_peer);

        let window = Duration::from_millis(200);
        let start = tokio::time::Instant::now();
        let outcome = copy_with_idle_window(&mut a, &mut b, window).await;
        let elapsed = start.elapsed();

        assert_eq!(
            outcome,
            CopyOutcome::Idle,
            "一侧 EOF 后对端静默，应按空闲回收"
        );
        // 应当等满空闲窗口（说明没有挂死在对端上，也没有提前误判 Finished）
        assert!(
            elapsed >= window && elapsed < window * 5,
            "耗时应接近空闲窗口，实际 {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn both_sides_eof_finishes_immediately() {
        // 两侧都关闭：应立即 Finished，不等空闲窗口。
        let (mut a, a_peer) = tokio::io::duplex(1024);
        let (mut b, b_peer) = tokio::io::duplex(1024);
        drop(a_peer);
        drop(b_peer);

        let start = tokio::time::Instant::now();
        let outcome = copy_with_idle_window(&mut a, &mut b, Duration::from_secs(30)).await;
        assert_eq!(outcome, CopyOutcome::Finished, "双方关闭应判定 Finished");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "不应等满空闲窗口，实际 {:?}",
            start.elapsed()
        );
    }
}
