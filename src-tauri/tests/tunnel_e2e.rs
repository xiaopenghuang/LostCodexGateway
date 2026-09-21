//! 集成测试：用真实产品代码（ssh 模块 + verify 模块）连 Docker sshd 夹具。
//! 前置条件：tests/fixtures/ssh-server/setup.ps1 已运行（容器 lcfg-test-sshd 在 127.0.0.1:2222）。
//! 运行：cargo test --test tunnel_e2e -- --ignored --nocapture
//!
//! 夹具网络 lcfg-test-net 内的 lcfg-test-web 只对容器网络可见：
//! 宿主机直连不通，只有经 SSH 隧道（服务器端建连）才能访问 —— 决定性证据。
//! 测试不会修改主机 ssh/known_hosts/代理配置；仅使用夹具专用私钥与临时端口。

use lostcodexgateway_lib::config::ServerConfig;
use lostcodexgateway_lib::ssh::{self, SshErrorClass, TunnelProcess};
use lostcodexgateway_lib::verify;

const FIXTURE_KEY: &str = r"..\tests\fixtures\ssh-server\keys\id_test_ed25519";
const SSH_EXE: &str = r"C:\Windows\System32\OpenSSH\ssh.exe";
const EARLY_WAIT_MS: u64 = 6000;

fn fixture_cfg(socks_port: u16) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".into(),
        port: 2222,
        username: "testuser".into(),
        key_path: FIXTURE_KEY.into(),
        socks_port,
        ssh_exe_path: SSH_EXE.into(),
        server_name: "docker-fixture".into(),
    }
}

async fn assert_tunnel_alive(port: u16) {
    for _ in 0..30 {
        if verify::port_listening(port) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    panic!("隧道端口 {} 未在预期时间内监听", port);
}

/// 收集早期错误直到出现致命分类或超时；返回 None 表示期间未出现致命错误。
async fn collect_fatal(
    tp: &mut TunnelProcess,
    timeout: std::time::Duration,
) -> Option<SshErrorClass> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, tp.stderr_rx.recv()).await {
            Ok(Some(line)) => {
                let cls = ssh::classify_stderr(&line);
                if !matches!(cls, SshErrorClass::Other(_)) {
                    return Some(cls);
                }
            }
            Ok(None) | Err(_) => return None,
        }
    }
}

/// RAII 清理：panic 时也保证子进程被终止。
struct TunnelGuard {
    tp: Option<TunnelProcess>,
}
impl TunnelGuard {
    async fn new(ssh_exe: &str, cfg: &ServerConfig) -> Self {
        match ssh::spawn_tunnel(ssh_exe, cfg) {
            Ok((tp, _abort)) => Self { tp: Some(tp) },
            Err(_) => Self { tp: None },
        }
    }
    fn inner(&mut self) -> &mut TunnelProcess {
        self.tp.as_mut().expect("隧道未启动")
    }
}
impl Drop for TunnelGuard {
    fn drop(&mut self) {
        if let Some(tp) = self.tp.as_mut() {
            ssh::stop_tunnel(tp);
            // 无法在 Drop 中 await，用同步等待回收（Windows 下 kill 后立即退出）
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
}

/// 核心验收：隧道建立 → 远端 DNS 访问「只有服务器可达」的内部服务（决定性证据）。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn e2e_tunnel_remote_dns_reaches_server_only_service() {
    let cfg = fixture_cfg(17811);
    let mut guard = TunnelGuard::new(SSH_EXE, &cfg).await;
    let tp = guard.inner();
    assert!(tp.pid > 0, "隧道进程未启动");
    assert_tunnel_alive(cfg.socks_port).await;

    // 决定性验证：远端 DNS 解析 lcfg-test-web（宿主机解析不了的名字）
    let mut last_err = String::new();
    let mut ok = false;
    for _ in 0..6 {
        match verify::fetch_http_via_socks(cfg.socks_port, "http://lcfg-test-web:8080/", 15).await {
            Ok((status, first)) if status == 200 && first == "LCFG-TUNNEL-OK" => {
                ok = true;
                break;
            }
            Ok((status, first)) => {
                last_err = format!("status={} first={:?}", status, first);
            }
            Err(e) => {
                last_err = e;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(ok, "经隧道访问内部服务失败（最后错误: {}）", last_err);

    // 补充：出口 IP 验证序列（端口监听 + SOCKS 握手 + 出口 IP）
    let result = verify::verify_tunnel(
        cfg.socks_port,
        &[
            "https://ifconfig.me/ip".to_string(),
            "https://api.ipify.org?format=json".to_string(),
        ],
        20,
    )
    .await;
    assert!(result.ok, "出口验证失败: {:?}", result.steps);
    assert!(result.egress_ip.is_some(), "出口 IP 为空");

    ssh::stop_tunnel(tp);
    ssh::wait_tunnel(tp).await;
    assert!(!verify::port_listening(cfg.socks_port), "停止后端口仍监听");
}

/// 错误场景：不存在的密钥 → AuthFailed 分类。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn e2e_wrong_key_classified_auth_failed() {
    let mut cfg = fixture_cfg(17812);
    cfg.key_path = "C:\\nonexistent\\no_key_here".into();
    let mut guard = TunnelGuard::new(SSH_EXE, &cfg).await;
    let tp = guard.inner();
    let cls = collect_fatal(tp, std::time::Duration::from_millis(EARLY_WAIT_MS)).await;
    ssh::stop_tunnel(tp);
    ssh::wait_tunnel(tp).await;
    assert_eq!(cls, Some(SshErrorClass::AuthFailed));
}

/// 错误场景：错误端口 → HostUnreachable（connection refused）分类。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn e2e_wrong_port_classified_unreachable() {
    let mut cfg = fixture_cfg(17813);
    cfg.port = 2223;
    let mut guard = TunnelGuard::new(SSH_EXE, &cfg).await;
    let tp = guard.inner();
    let cls = collect_fatal(tp, std::time::Duration::from_millis(EARLY_WAIT_MS)).await;
    ssh::stop_tunnel(tp);
    ssh::wait_tunnel(tp).await;
    assert_eq!(cls, Some(SshErrorClass::HostUnreachable));
}

/// 错误场景：DNS 失败 → DnsFailed 分类。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn e2e_dns_failure_classified() {
    let mut cfg = fixture_cfg(17814);
    cfg.host = "nonexistent-domain-lcfg.invalid".into();
    let mut guard = TunnelGuard::new(SSH_EXE, &cfg).await;
    let tp = guard.inner();
    let cls = collect_fatal(tp, std::time::Duration::from_millis(EARLY_WAIT_MS)).await;
    ssh::stop_tunnel(tp);
    ssh::wait_tunnel(tp).await;
    assert_eq!(cls, Some(SshErrorClass::DnsFailed));
}

/// 错误场景：本地端口被占用 → LocalPortBusy 分类。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn e2e_local_port_busy_classified() {
    let cfg = fixture_cfg(17815);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:17815")
        .await
        .expect("先占用端口");
    let mut guard = TunnelGuard::new(SSH_EXE, &cfg).await;
    let tp = guard.inner();
    let cls = collect_fatal(tp, std::time::Duration::from_millis(EARLY_WAIT_MS)).await;
    ssh::stop_tunnel(tp);
    ssh::wait_tunnel(tp).await;
    drop(listener);
    assert_eq!(cls, Some(SshErrorClass::LocalPortBusy));
}
