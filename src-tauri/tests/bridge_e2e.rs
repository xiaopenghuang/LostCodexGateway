//! 桥接层集成测试：HTTP CONNECT 桥接 → 隧道 → 内部服务（决定性）。
//! 用原始 TCP 发 CONNECT（与 Codex/reqwest 对 HTTPS 目标的用法一致），
//! 全流程带硬超时，测试结束保证清理子进程。
//! 运行：cargo test --test bridge_e2e -- --ignored --nocapture

use lostcodexgateway_lib::bridge;
use lostcodexgateway_lib::config::ServerConfig;
use lostcodexgateway_lib::ssh;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const FIXTURE_KEY: &str = r"..\tests\fixtures\ssh-server\keys\id_test_ed25519";
const SSH_EXE: &str = r"C:\Windows\System32\OpenSSH\ssh.exe";

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

async fn expect_port(port: u16, desc: &str) {
    for _ in 0..30 {
        if lostcodexgateway_lib::verify::port_listening(port) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("{} 端口 {} 未监听", desc, port);
}

/// 原始 HTTP CONNECT 请求：连 bridge → CONNECT 内部域名 → 200 后发 GET → 校验响应。
async fn raw_connect_request(bridge_port: u16) -> Result<String, String> {
    let mut s = tokio::time::timeout(
        Duration::from_secs(10),
        TcpStream::connect(("127.0.0.1", bridge_port)),
    )
    .await
    .map_err(|_| "连桥接超时".to_string())?
    .map_err(|e| e.to_string())?;

    // CONNECT（模拟 HTTPS 代理语义；Codex 实测全部目标走 CONNECT）
    s.write_all(b"CONNECT lcfg-test-web:8080 HTTP/1.1\r\nHost: lcfg-test-web:8080\r\n\r\n")
        .await
        .map_err(|e| e.to_string())?;
    // 读响应头
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(10), s.read(&mut tmp))
            .await
            .map_err(|_| "读 200 头超时".to_string())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("桥接在 CONNECT 阶段关闭连接".to_string());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 4096 {
            return Err("响应头超长".to_string());
        }
    }
    let head = String::from_utf8_lossy(&buf);
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("CONNECT 未返回 200: {}", head.lines().next().unwrap_or("")));
    }

    // 通过隧道发 GET
    s.write_all(b"GET / HTTP/1.1\r\nHost: lcfg-test-web\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    loop {
        let n = tokio::time::timeout(Duration::from_secs(10), s.read(&mut tmp))
            .await
            .map_err(|_| "读响应体超时".to_string())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
        if body.len() > 64 * 1024 {
            return Err("响应体超长".to_string());
        }
    }
    Ok(String::from_utf8_lossy(&body).to_string())
}

#[tokio::test]
#[ignore = "需要 Docker 夹具（tests/fixtures/ssh-server/setup.ps1）"]
async fn bridge_reaches_internal_service_via_tunnel() {
    let cfg = fixture_cfg(17821);
    let (mut tp, _abort) = ssh::spawn_tunnel(SSH_EXE, &cfg).expect("隧道启动失败");
    let result: Result<(), String> = async {
        expect_port(cfg.socks_port, "隧道").await;

        let (bridge_port, _task, stats) =
            bridge::start_bridge(cfg.socks_port).await.map_err(|e| e)?;
        assert!(bridge_port > 0);
        expect_port(bridge_port, "桥接").await;

        let body = raw_connect_request(bridge_port).await?;
        assert!(body.contains("LCFG-TUNNEL-OK"), "响应体不含标记: {:?}", &body[..body.len().min(120)]);

        let snap = stats.snapshot();
        assert!(snap.connections_total >= 1, "桥接无连接记录");
        assert_eq!(snap.last_target.as_deref(), Some("lcfg-test-web:8080"));
        Ok(())
    }
    .await;

    ssh::stop_tunnel(&mut tp);
    ssh::wait_tunnel(&mut tp).await;
    result.expect("桥接端到端失败");
    assert!(!lostcodexgateway_lib::verify::port_listening(cfg.socks_port));
}

// ---------------------------------------------------------------------------
// 安全拒绝路径（不需要隧道 / 不需要 Docker，可常规运行）
//
// 这一组测试针对「桥接层自己声明的安全约束」，用真实 socket 验证从 accept 到
// 返回码的完整链路——只测 check_target 谓词是不够的，谓词对了但没接线一样是漏洞。
// ---------------------------------------------------------------------------

/// 起一个假的 SOCKS5 服务器，用来观察桥接层是否真的把请求转发下去了。
///
/// 返回 (端口, 收到的 CONNECT 目标列表句柄)。
/// 若桥接层在转发前就拒绝了，这个列表应当保持为空——这正是我们要断言的关键点。
async fn spawn_fake_socks5() -> (u16, std::sync::Arc<parking_lot::Mutex<Vec<String>>>) {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                break;
            };
            let seen = seen2.clone();
            tokio::spawn(async move {
                // SOCKS5 无认证握手
                let mut h = [0u8; 3];
                if s.read_exact(&mut h).await.is_err() {
                    return;
                }
                if s.write_all(&[0x05, 0x00]).await.is_err() {
                    return;
                }
                // CONNECT 请求
                let mut head = [0u8; 4];
                if s.read_exact(&mut head).await.is_err() {
                    return;
                }
                let mut target = String::new();
                match head[3] {
                    0x01 => {
                        let mut a = [0u8; 4];
                        let _ = s.read_exact(&mut a).await;
                        target = format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
                    }
                    0x03 => {
                        let mut l = [0u8; 1];
                        let _ = s.read_exact(&mut l).await;
                        let mut d = vec![0u8; l[0] as usize];
                        let _ = s.read_exact(&mut d).await;
                        target = String::from_utf8_lossy(&d).to_string();
                    }
                    _ => {}
                }
                let mut p = [0u8; 2];
                let _ = s.read_exact(&mut p).await;
                let port = u16::from_be_bytes(p);
                seen.lock().push(format!("{}:{}", target, port));
                // 回成功（假装连上了）
                let _ = s
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 1])
                    .await;
                // 保持连接直到对端关闭
                let mut sink = [0u8; 1024];
                loop {
                    match s.read(&mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
        }
    });
    (port, seen)
}

/// 发一个原始 CONNECT 请求，返回桥接层的响应头文本。
async fn send_connect(bridge_port: u16, request: &str) -> String {
    let mut s = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", bridge_port)),
    )
    .await
    .expect("连桥接超时")
    .expect("连桥接失败");
    s.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 512];
    loop {
        let n = match tokio::time::timeout(Duration::from_secs(5), s.read(&mut tmp)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => n,
            Ok(Err(_)) => break,
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 4096 {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_rejects_loopback_and_private_targets_without_forwarding() {
    let (socks_port, seen) = spawn_fake_socks5().await;
    let (bridge_port, _task, stats) = bridge::start_bridge(socks_port).await.expect("桥接启动失败");

    // 1) 回环目标 → 403 + BRIDGE_LOOPBACK_TARGET
    let r = send_connect(bridge_port, "CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 403"), "回环目标应 403，实际: {}", r);
    assert!(r.contains("BRIDGE_LOOPBACK_TARGET"), "应带错误码，实际: {}", r);

    // 2) localhost 同样拒绝
    let r = send_connect(bridge_port, "CONNECT localhost:443 HTTP/1.1\r\n\r\n").await;
    assert!(r.contains("BRIDGE_LOOPBACK_TARGET"), "localhost 应拒绝，实际: {}", r);

    // 3) 私有网段 → 403 + BRIDGE_PRIVATE_TARGET
    let r = send_connect(bridge_port, "CONNECT 192.168.1.1:443 HTTP/1.1\r\n\r\n").await;
    assert!(r.contains("BRIDGE_PRIVATE_TARGET"), "私有网段应拒绝，实际: {}", r);

    // 4) 非 CONNECT 方法 → 400
    let r = send_connect(bridge_port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 400"), "非 CONNECT 应 400，实际: {}", r);

    // 5) 关键断言：以上请求一个都不应到达下游 SOCKS5
    //    （「拒绝了」和「拒绝得干净」是两回事——如果先转发再拒绝，
    //     内网探测已经发生了，等于没防住）
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        seen.lock().is_empty(),
        "被拒绝的请求竟然到达了上游 SOCKS5: {:?}",
        seen.lock()
    );

    // 6) 拒绝计数与记录应可查（供诊断页展示）
    let snap = stats.snapshot();
    assert!(snap.rejects_total >= 4, "拒绝计数应累加，实际 {}", snap.rejects_total);
    assert!(!snap.recent_rejects.is_empty());
    assert!(snap.recent_rejects.iter().all(|r| r.code.starts_with("BRIDGE_")));
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_rejects_self_target_to_prevent_proxy_loop() {
    let (socks_port, _seen) = spawn_fake_socks5().await;
    let (bridge_port, _task, stats) = bridge::start_bridge(socks_port).await.expect("桥接启动失败");

    // 目标恰好是桥接层自己的端口 → 必须判为 SelfTarget（否则会形成
    // 桥接 → SOCKS → ssh → … → 回到桥接 的循环）
    let req = format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", bridge_port);
    let r = send_connect(bridge_port, &req).await;
    // 回环判定优先于 SelfTarget，两者都算正确拦截；此处只要求被拒绝
    assert!(r.starts_with("HTTP/1.1 403"), "自身端口应被拒绝，实际: {}", r);
    assert!(stats.snapshot().rejects_total >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_forwards_public_target_and_records_it() {
    let (socks_port, seen) = spawn_fake_socks5().await;
    let (bridge_port, _task, stats) = bridge::start_bridge(socks_port).await.expect("桥接启动失败");

    // 公网域名：应放行并真实转发到下游 SOCKS5（远端 DNS 语义）
    let r = send_connect(bridge_port, "CONNECT api.github.com:443 HTTP/1.1\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 200"), "公网目标应 200，实际: {}", r);

    // 等到假 SOCKS5 收到请求
    for _ in 0..20 {
        if !seen.lock().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        seen.lock().first().map(|s| s.as_str()),
        Some("api.github.com:443"),
        "下游应收到原样域名（远端 DNS），实际: {:?}",
        seen.lock()
    );

    let snap = stats.snapshot();
    assert_eq!(snap.connections_total, 1);
    assert_eq!(snap.last_target.as_deref(), Some("api.github.com:443"));
    assert_eq!(snap.rejects_total, 0, "正常请求不应计入拒绝");
}

// ---------------------------------------------------------------------------
// 空闲超时的语义（不需要隧道）
// ---------------------------------------------------------------------------

/// 验证「持续有数据流动的连接不会被空闲超时误杀」。
///
/// 这是加固阶段修掉的一个真实缺陷：最初用 `timeout(idle, copy_bidirectional(..))`，
/// 那是**总时长**上限，会把长连接在固定时间点硬切断。正确的语义是「空闲」——
/// 有字节流动就重新计时。
///
/// 本测试的做法：上游持续吐数据，断言客户端能**持续**收到数据（不依赖精确字节数，
/// 避免并行测试时 CPU 争抢导致 flaky）。若实现是总时长上限，流的长度会有上限；
/// 若是正确的空闲语义，只要有数据在流就永远不断。
#[tokio::test(flavor = "multi_thread")]
async fn idle_timeout_does_not_kill_active_connection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (socks_port, seen) = spawn_passthrough_socks5().await;
    let (bridge_port, _task, _stats) = bridge::start_bridge(socks_port).await.expect("桥接启动失败");

    // 用公网形式的目标让桥接放行（假 SOCKS5 会按目标名回一个活跃数据流）
    let mut s = TcpStream::connect(("127.0.0.1", bridge_port)).await.unwrap();
    s.write_all(b"CONNECT stream.example.com:443 HTTP/1.1\r\n\r\n")
        .await
        .unwrap();

    // 读 200
    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf))
        .await
        .expect("读 200 超时")
        .unwrap();
    let head = String::from_utf8_lossy(&buf[..n]);
    assert!(head.starts_with("HTTP/1.1 200"), "应建立隧道: {}", head);
    assert!(!seen.lock().is_empty());

    // 关键断言：分两个时间段各收一次数据，中间**不**让连接空闲超过空闲窗口，
    // 从而验证计时器确实被数据流重置（而不是按总时长一刀切）。
    // 用「两次都收到」作为判据，比精确字节数更抗并行干扰。
    let mut rounds = 0;
    for _ in 0..2 {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut got_any = false;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), s.read(&mut buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => {
                    got_any = true;
                    break;
                }
                Ok(Err(_)) => break,
                Err(_) => {}
            }
        }
        assert!(got_any, "第 {} 轮未收到数据，活跃连接疑似被误杀", rounds + 1);
        rounds += 1;
    }
    assert_eq!(rounds, 2);
}

/// 假 SOCKS5：忽略目标地址，一律连到预置的上游端口并双向转发。
/// （生产桥接层不做这种事；这里仅用于模拟下游可达。）
async fn spawn_passthrough_socks5() -> (u16, std::sync::Arc<parking_lot::Mutex<Vec<String>>>) {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen2.clone();
            tokio::spawn(async move {
                let mut h = [0u8; 3];
                if s.read_exact(&mut h).await.is_err() {
                    return;
                }
                if s.write_all(&[0x05, 0x00]).await.is_err() {
                    return;
                }
                let mut head = [0u8; 4];
                if s.read_exact(&mut head).await.is_err() {
                    return;
                }
                // 读掉地址与端口
                match head[3] {
                    0x01 => {
                        let mut a = [0u8; 6];
                        let _ = s.read_exact(&mut a).await;
                    }
                    0x03 => {
                        let mut l = [0u8; 1];
                        let _ = s.read_exact(&mut l).await;
                        let mut d = vec![0u8; l[0] as usize + 2];
                        let _ = s.read_exact(&mut d).await;
                        seen.lock().push(String::from_utf8_lossy(&d[..d.len() - 2]).to_string());
                    }
                    _ => return,
                }
                // 回成功，然后持续吐数据：只要有数据在流，桥接层就不应判定空闲。
                // 用「每 50ms 一个字节」而不是固定总字节数，避免依赖精确时序。
                if s.write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 1])
                    .await
                    .is_err()
                {
                    return;
                }
                loop {
                    if s.write_all(b"x").await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            });
        }
    });
    (port, seen)
}
