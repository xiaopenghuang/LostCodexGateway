//! Codex CLI 专用启动器（M2）：
//! - 定位官方 codex 入口（codex.cmd / 原生 codex.exe）
//! - 只向新子进程注入代理环境变量（不写用户/系统环境）
//! - 实测结论（tests/e2e 记录）：Codex 原生二进制读取 HTTP_PROXY/HTTPS_PROXY，
//!   不接受 socks5:// 直供 → 必须经 HTTP CONNECT 桥接层。

use crate::config::GatewayConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchPreview {
    pub command: String,
    pub env: Vec<(String, String)>,
    pub proxy_line: String,
    pub bridge_needed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchResult {
    pub started: bool,
    pub message: String,
    pub pid: Option<u32>,
}

/// 定位 Codex CLI 入口。返回 (说明文本, 可执行路径, 是否 codex.cmd shim)。
pub fn locate_codex() -> Option<(String, PathBuf, bool)> {
    // 优先 PATH 中的 codex.cmd（npm 全局 shim → 自动定位原生二进制）
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(';') {
            if dir.trim().is_empty() {
                continue;
            }
            let cmd = PathBuf::from(dir).join("codex.cmd");
            if cmd.exists() {
                return Some(("codex.cmd (npm shim)".to_string(), cmd, true));
            }
        }
    }
    // npm 全局目录：%APPDATA%\npm\node_modules
    for (base_label, base) in npm_global_dirs() {
        let cmd = PathBuf::from(&base).join("codex.cmd");
        if cmd.exists() {
            return Some((format!("codex.cmd ({})", base_label), cmd, true));
        }
        // 原生二进制的常见落位（npm 全局安装 + 独立安装）
        let bin_rel = r"node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe";
        let vendor_rel = r"node_modules\@openai\codex\vendor\x86_64-pc-windows-msvc\bin\codex.exe";
        for rel in [bin_rel, vendor_rel] {
            let p = PathBuf::from(&base).join(rel);
            if p.exists() {
                return Some(("codex.exe (npm 全局原生二进制)".to_string(), p, false));
            }
        }
    }
    // 独立安装：%LOCALAPPDATA%\Programs\codex
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&local)
            .join("Programs")
            .join("codex")
            .join("codex.exe");
        if p.exists() {
            return Some(("codex.exe (独立安装)".to_string(), p, false));
        }
    }
    None
}

/// 候选 npm 全局根目录（不含任何写死的个人路径）。
fn npm_global_dirs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            out.push(("%APPDATA%\\npm".to_string(), PathBuf::from(&appdata).join("npm").to_string_lossy().to_string()));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.trim().is_empty() {
            // 部分环境 npm prefix 指向 LOCALAPPDATA
            out.push((
                "%LOCALAPPDATA%\\npm".to_string(),
                PathBuf::from(&local).join("npm").to_string_lossy().to_string(),
            ));
        }
    }
    out
}

/// 生成启动预览（桥接端口由调用方给出）。
pub fn build_preview(cfg: &GatewayConfig, bridge_port: Option<u16>) -> LaunchPreview {
    let (_label, exe, is_shim) = match locate_codex() {
        Some(v) => v,
        None => {
            return LaunchPreview {
                command: "未找到 Codex CLI".to_string(),
                env: vec![],
                proxy_line: "不可用".to_string(),
                bridge_needed: true,
            }
        }
    };
    let mut env: Vec<(String, String)> = vec![
        (
            "NO_PROXY".to_string(),
            "localhost,127.0.0.1,::1".to_string(),
        ),
        (
            "no_proxy".to_string(),
            "localhost,127.0.0.1,::1".to_string(),
        ),
    ];
    let proxy_line = match bridge_port {
        Some(port) => {
            let http_proxy = format!("http://127.0.0.1:{}", port);
            env.push(("HTTP_PROXY".to_string(), http_proxy.clone()));
            env.push(("HTTPS_PROXY".to_string(), http_proxy.clone()));
            env.push(("http_proxy".to_string(), http_proxy.clone()));
            env.push(("https_proxy".to_string(), http_proxy.clone()));
            format!("HTTP CONNECT 桥接层 127.0.0.1:{} → SOCKS5 {}（远端 DNS）", port, cfg.server.socks_port)
        }
        None => "桥接层未运行（不可启动）".to_string(),
    };
    let _ = is_shim;
    LaunchPreview {
        command: format!("powershell.exe -NoProfile -Command \"...env 注入...; & '{}'\"", exe.display()),
        env,
        proxy_line,
        bridge_needed: true,
    }
}

/// 启动 Codex CLI：独立 PowerShell 子进程，仅向该子进程注入代理环境变量。
/// 使用参数数组，无 shell 拼接。
pub fn launch_cli(cfg: &GatewayConfig, bridge_port: u16) -> Result<LaunchResult, String> {
    let (_label, exe, _is_shim) =
        locate_codex().ok_or_else(|| "未找到 Codex CLI（请先安装官方 codex）".to_string())?;
    let proxy = format!("http://127.0.0.1:{}", bridge_port);

    // 构造 PowerShell 命令文本（全部为固定模板 + 变量插入；变量来自本工具配置，
    // 无用户 shell 注入面：路径用单引号包裹并转义单引号）
    let exe_quoted = exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "$env:HTTP_PROXY='{}'; $env:HTTPS_PROXY='{}'; $env:http_proxy='{}'; $env:https_proxy='{}'; \
         $env:NO_PROXY='localhost,127.0.0.1,::1'; $env:no_proxy='localhost,127.0.0.1,::1'; \
         Write-Host '[LostCodexGateway] 代理已注入本终端：HTTP_PROXY={}' ; \
         & '{}'",
        proxy, proxy, proxy, proxy, proxy, exe_quoted
    );
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-NoExit")
        .arg("-Command")
        .arg(&ps)
        .creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE：独立终端窗口
    let child = cmd.spawn().map_err(|e| format!("启动 PowerShell 失败: {}", e))?;
    let pid = child.id().unwrap_or(0);
    let _ = cfg;
    Ok(LaunchResult {
        started: pid > 0,
        message: if pid > 0 {
            "已在新终端启动 Codex CLI（代理仅注入该进程）".to_string()
        } else {
            "启动失败：未获得进程 ID".to_string()
        },
        pid: if pid > 0 { Some(pid) } else { None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_requires_bridge() {
        let cfg = GatewayConfig::default();
        let p = build_preview(&cfg, None);
        assert!(p.bridge_needed);
        assert!(p.env.iter().any(|(k, _)| k == "NO_PROXY"));
    }

    #[test]
    fn preview_with_bridge_has_http_proxy() {
        let mut cfg = GatewayConfig::default();
        cfg.server.socks_port = 17801;
        let p = build_preview(&cfg, Some(18999));
        assert!(p.env.iter().any(|(k, v)| k == "HTTP_PROXY" && v == "http://127.0.0.1:18999"));
        assert!(p.env.iter().any(|(k, v)| k == "HTTPS_PROXY" && v == "http://127.0.0.1:18999"));
        // 绝不允许把 SOCKS 地址当 HTTP 代理注入
        assert!(!p.env.iter().any(|(_, v)| v.contains("socks5://")));
    }
}
