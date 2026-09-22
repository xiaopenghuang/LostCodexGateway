//! 网络诊断集成测试（真实 Docker 夹具）：
//! 验收点 1：SSH 已连接但 SOCKS 无法转发时，能识别异常（隧道异常状态）
//! 验收点 2：本地出口与网关出口独立检测（网关出口必须经 SOCKS）
//! 验收点 7：服务器只读检测可用；不修改服务器配置
//! 运行：cargo test --test diag_e2e -- --ignored --test-threads=1 --nocapture

use lostcodexgateway_lib::config::{GatewayConfig, ServerProfile};
use lostcodexgateway_lib::diagnostics::{self, DiagStatus};
use lostcodexgateway_lib::ssh;
use lostcodexgateway_lib::state::{GatewayStateMachine};
use std::time::Duration;

const FIXTURE_KEY: &str = r"..\tests\fixtures\ssh-server\keys\id_test_ed25519";
const SSH_EXE: &str = r"C:\Windows\System32\OpenSSH\ssh.exe";

/// 夹具配置 + 本次测试使用的 SOCKS 端口（端口是全局设置，不再是服务器字段）。
fn fixture(socks_port: u16) -> (GatewayConfig, u16) {
    let mut cfg = GatewayConfig::default();
    cfg.servers = vec![ServerProfile {
        id: "fixture".into(),
        name: "docker-fixture".into(),
        host: "127.0.0.1".into(),
        port: 2222,
        username: "testuser".into(),
        key_path: FIXTURE_KEY.into(),
        ssh_exe_path: SSH_EXE.into(),
        ..Default::default()
    }];
    cfg.active_server_id = "fixture".into();
    cfg.settings.socks_port = socks_port;
    cfg.normalize();
    (cfg, socks_port)
}

async fn expect_port(port: u16) {
    for _ in 0..30 {
        if lostcodexgateway_lib::verify::port_listening(port) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("端口 {} 未监听", port);
}

/// 验收点 1+2+7：正常隧道下的完整诊断。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn diag_full_with_healthy_tunnel() {
    let (cfg, socks_port) = fixture(17831);
    let machine = GatewayStateMachine::new(cfg.clone());
    // 启动隧道并记录 pid
    let server = cfg.active_server().expect("夹具应有一台服务器").clone();
    let (mut tp, _abort) = ssh::spawn_tunnel(SSH_EXE, &server, socks_port).expect("隧道启动失败");
    expect_port(socks_port).await;
    {
        let mut inner = machine.inner.lock();
        inner.tunnel_pid = Some(tp.pid);
        inner.state = lostcodexgateway_lib::state::GatewayState::EgressVerified;
    }

    let report = diagnostics::run_full_diagnostics(&machine.inner, None).await;

    // 隧道四项全 OK
    for t in &report.tunnel_items {
        assert_eq!(t.status, DiagStatus::Ok, "隧道项 {} 异常: {}", t.key, t.detail);
    }
    assert_eq!(report.tunnel_status, DiagStatus::Ok);

    // 网关出口必须存在（经 SOCKS 远端 DNS 成功）
    assert!(report.egress.gateway_ip.is_some(), "网关出口为空: {:?}", report.egress.items);
    assert_eq!(report.egress.match_result, "unconfirmed"); // 未配置预期 IP

    // 服务器只读检测可达
    assert!(report.server.reachable, "服务器检测失败: {:?}", report.server.items);

    // 路径四跳齐全
    assert_eq!(report.path_hops.len(), 4);

    ssh::stop_tunnel(&mut tp);
    ssh::wait_tunnel(&mut tp).await;
}

/// 验收点 1 变体：进程存活但端口被占 → 检测必须报隧道异常而非「正常」。
/// 这里用「隧道未启动」+ state 仍显示已验证的组合模拟误报场景：
/// 实际上直接验证：无隧道时诊断必须如实报错。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn diag_detects_dead_tunnel() {
    let (cfg, _socks_port) = fixture(17832);
    let machine = GatewayStateMachine::new(cfg.clone());
    {
        let mut inner = machine.inner.lock();
        // 伪造「UI 认为已连接」但实际无隧道（模拟进程退出后状态未及时同步的场景）
        inner.state = lostcodexgateway_lib::state::GatewayState::EgressVerified;
        inner.tunnel_pid = Some(999999); // 不存在的 PID
    }

    let report = diagnostics::run_full_diagnostics(&machine.inner, None).await;

    // 端口无监听 → Error
    let port_item = report.tunnel_items.iter().find(|i| i.key == "port_listen").unwrap();
    assert_eq!(port_item.status, DiagStatus::Error);
    // PID 已退出 → Error
    let pid_item = report.tunnel_items.iter().find(|i| i.key == "ssh_pid").unwrap();
    assert_eq!(pid_item.status, DiagStatus::Error);
    // 整体不可能是 Ok
    assert_ne!(report.tunnel_status, DiagStatus::Ok);
    // 网关出口为空（不虚报）
    assert!(report.egress.gateway_ip.is_none());
}

/// 服务器只读检测：不修改任何服务器配置（对比检测前后 sshd_config 哈希）。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn server_diag_is_read_only() {
    let (cfg, _socks_port) = fixture(17833);
    let before = ssh_config_hash();
    let (ms, out) = diagnostics::run_server_diag(&cfg, SSH_EXE)
        .await
        .expect("服务器检测失败");
    assert!(ms > 0);
    assert!(out.contains("TCP_OK") || out.contains("TCP_FAIL"));
    assert!(out.contains("HTTPS_OK") || out.contains("HTTPS_FAIL"));
    let after = ssh_config_hash();
    assert_eq!(before, after, "服务器 sshd_config 被修改！");
}

fn ssh_config_hash() -> String {
    // 只读方式获取容器内 sshd_config 的 sha256（docker exec，不改文件）
    let out = std::process::Command::new("docker")
        .args([
            "exec",
            "lcfg-test-sshd",
            "sha256sum",
            "/etc/ssh/sshd_config",
        ])
        .output()
        .expect("docker exec 失败");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 预期出口 IP 配置：设错 IP → mismatch（验收点：出口不匹配时提示）。
#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn egress_mismatch_detected() {
    let (mut cfg, socks_port) = fixture(17834);
    // 预期出口 IP 现在是**每台服务器**的字段（不再是全局 verify）
    cfg.servers[0].expected_egress_ip = "203.0.113.99".to_string(); // 故意错误（TEST-NET-3）
    let machine = GatewayStateMachine::new(cfg.clone());
    let server = cfg.active_server().expect("夹具应有一台服务器").clone();
    let (mut tp, _abort) = ssh::spawn_tunnel(SSH_EXE, &server, socks_port).expect("隧道启动失败");
    expect_port(socks_port).await;
    {
        let mut inner = machine.inner.lock();
        inner.tunnel_pid = Some(tp.pid);
        inner.state = lostcodexgateway_lib::state::GatewayState::EgressVerified;
    }

    let report = diagnostics::run_full_diagnostics(&machine.inner, None).await;
    assert_eq!(report.egress.match_result, "mismatch");
    assert!(report.advisories.iter().any(|a| a.contains("出口 IP 不匹配")));

    ssh::stop_tunnel(&mut tp);
    ssh::wait_tunnel(&mut tp).await;
}
