//! Mihomo / Clash Verge Rev 受控集成（M3）：
//! - 只读检测：Verge 版本、mihomo 进程、mixed 端口、external-controller、TUN 状态、profile 结构
//! - 规则片段生成：PROCESS-NAME/PROCESS-PATH → MY-VPS，置于最终 MATCH 之前（片段文件，不自动写入）
//! - 备份/回滚：只备份本工具指定的文件；回滚仅恢复本工具的备份，绝不覆盖用户新改动
//!
//! 安全边界（文档 §5.4）：
//! - 不直接改写在线订阅文件；不假定 controller 能永久修改 Verge 管理的 profile
//! - 通用进程（Code.exe/node.exe 等）不得全量导流；TUN 未开启时如实报告「尚未覆盖」

use crate::procutil::std_cmd;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MihomoDetection {
    pub verge_installed: bool,
    pub verge_version: Option<String>,
    /// 探测到的 Verge 可执行文件路径（找不到时为 None，不猜路径）。
    pub verge_path: Option<String>,
    pub verge_running: bool,
    pub mihomo_running: bool,
    pub mixed_port: Option<u16>,
    pub external_controller: Option<String>,
    pub tun_enabled: bool,
    pub mode: Option<String>,
    pub profiles_dir: Option<String>,
    pub config_dir: Option<String>,
    pub notes: Vec<String>,
}

/// Verge 数据目录（实测位置）。
pub fn verge_data_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(&appdata).join("io.github.clash-verge-rev.clash-verge-rev");
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&local).join("clash-verge-rev");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// 定位 Clash Verge Rev 主程序的真实路径（只读，不猜测）。
///
/// 顺序：注册表卸载信息 → 已运行进程的实际映像路径 → 常见安装目录 → PATH。
/// 全部失败返回 None——**宁可报告「未知版本」，不要猜一个路径**。
fn locate_verge_exe() -> Option<PathBuf> {
    // 1) 注册表：Verge 是 NSIS 安装，卸载项里通常带 DisplayIcon / InstallLocation
    if let Some(p) = from_registry() {
        return Some(p);
    }
    // 2) 已运行进程的映像路径（最可靠：进程真在跑就一定拿得到）
    if let Some(p) = from_running_process() {
        return Some(p);
    }
    // 3) 常见安装位置（含各盘符 Program Files）
    for cand in common_install_paths() {
        if cand.exists() {
            return Some(cand);
        }
    }
    // 4) PATH
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(';') {
            let p = PathBuf::from(dir).join(VERGE_EXE);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

const VERGE_EXE: &str = "clash-verge.exe";

/// 剥掉 DisplayIcon 值末尾的 `,<图标索引>` 后缀（`R:` 或 `R:,-1`）。
/// 引号由调用方在剥离后缀之后处理，避免残留尾部引号。
fn strip_icon_index(raw: &str) -> &str {
    match raw.rsplit_once(',') {
        Some((head, tail)) => {
            let t = tail.trim();
            if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '-') {
                head
            } else {
                raw
            }
        }
        None => raw,
    }
}

/// 从卸载注册表项解析 DisplayIcon（形如 `D:\Clash Verge\clash-verge.exe,0`）。
fn from_registry() -> Option<PathBuf> {
    let keys = [
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Clash Verge",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Clash Verge",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Clash Verge",
    ];
    for key in keys {
        let Ok(out) = std_cmd("reg").args(["query", key, "/v", "DisplayIcon"]).output() else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        // 值形如：    DisplayIcon    REG_SZ    D:\Clash Verge\clash-verge.exe,0
        //      或：    DisplayIcon    REG_SZ    "D:\Tools\Clash Verge\clash-verge.exe",-1
        // 注意顺序：必须先剥掉 `,<图标索引>`，再去引号。反过来做的话，
        // 带引号且以 `,-1` 结尾的值会留下一个多余的尾部引号。
        for line in text.lines() {
            let Some(idx) = line.find("REG_SZ") else { continue };
            let raw = line[idx + "REG_SZ".len()..].trim();
            let path = strip_icon_index(raw);
            let p = PathBuf::from(path.trim_matches('"').trim());
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// 从正在运行的 clash-verge.exe 进程读取映像路径。
fn from_running_process() -> Option<PathBuf> {
    // wmic 在新版 Windows 已弃用，优先 PowerShell CIM
    let out = std_cmd("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-Process -Name 'clash-verge' -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Path)",
        ])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    let p = PathBuf::from(path);
    p.exists().then_some(p)
}

/// 各盘符下的常见安装目录（覆盖 Program Files 与免安装解压目录）。
fn common_install_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut roots: Vec<String> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA", "USERPROFILE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                roots.push(v);
            }
        }
    }
    for root in roots {
        out.push(PathBuf::from(&root).join("Clash Verge").join(VERGE_EXE));
        // 免安装常见命名
        out.push(PathBuf::from(&root).join("clash-verge").join(VERGE_EXE));
    }
    // 盘符根的常见目录：只查存在盘符，不遍历全盘
    for drive in 'C'..='Z' {
        let root = format!("{}:\\", drive);
        if !PathBuf::from(&root).exists() {
            continue;
        }
        out.push(PathBuf::from(&root).join("Clash Verge").join(VERGE_EXE));
        out.push(PathBuf::from(&root).join("ClashVerge").join(VERGE_EXE));
    }
    out
}

/// 只读检测：不修改任何 Mihomo/Verge 配置。
pub fn detect() -> MihomoDetection {
    let mut d = MihomoDetection::default();
    let Some(data_dir) = verge_data_dir() else {
        d.notes.push("未找到 Clash Verge Rev 数据目录".to_string());
        return d;
    };
    d.verge_installed = true;
    d.config_dir = Some(data_dir.to_string_lossy().to_string());
    let profiles_dir = data_dir.join("profiles");
    if profiles_dir.exists() {
        d.profiles_dir = Some(profiles_dir.to_string_lossy().to_string());
    }

    // 进程检测（不读敏感内容）
    let output = std_cmd("tasklist")
        .args(["/FO", "CSV"])
        .output();
    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        d.verge_running = text.contains("clash-verge.exe");
        d.mihomo_running = text.contains("verge-mihomo.exe") || text.contains("mihomo.exe");
        if !d.verge_running && !d.mihomo_running {
            d.notes.push("Clash Verge 未运行".to_string());
        }
    }

    // verge.yaml：版本等元信息（键值只读）
    let verge_yaml = data_dir.join("verge.yaml");
    if let Ok(raw) = std::fs::read_to_string(&verge_yaml) {
        if let Some(v) = extract_key(&raw, "verge_mixed_port") {
            d.mixed_port = v.trim().parse().ok();
        }
    }
    // Verge 版本信息：先定位实际安装路径（用户可能装在任意盘符），再读文件版本。
    // 不硬编码路径——曾经写死 D:\Clash Verge\，装到别处就永远读不到版本。
    if let Some(exe) = locate_verge_exe() {
        d.verge_path = Some(exe.to_string_lossy().to_string());
        if let Ok(out) = std_cmd("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-Item -LiteralPath '{}' -ErrorAction SilentlyContinue).VersionInfo.FileVersion",
                    exe.to_string_lossy().replace('\'', "''")
                ),
            ])
            .output()
        {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !v.is_empty() {
                d.verge_version = Some(v);
            }
        }
    }

    // clash-verge.yaml（Verge 运行时合成配置）：mixed-port、external-controller、mode、tun.enable
    let runtime_cfg = data_dir.join("clash-verge.yaml");
    if let Ok(raw) = std::fs::read_to_string(&runtime_cfg) {
        d.mixed_port = d.mixed_port.or_else(|| extract_key(&raw, "mixed-port").and_then(|v| v.trim().parse().ok()));
        d.external_controller = extract_key(&raw, "external-controller");
        d.mode = extract_key(&raw, "mode");
        // tun 块（只提取 enable 布尔）
        if let Some(tun_block) = extract_yaml_block(&raw, "tun:") {
            d.tun_enabled = extract_key(&tun_block, "enable")
                .map(|v| v.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false);
        }
        if !d.tun_enabled {
            d.notes.push("TUN 未开启：Desktop/IDE 的进程级分流未覆盖（如实报告）".to_string());
        }
        // secret 存在性检测：只记录「有/无」，不读值
        if extract_key(&raw, "secret").is_some() {
            d.notes.push("external-controller 设置了 secret（本工具不读取、不保存该值）".to_string());
        }
    } else {
        d.notes.push("未找到运行时配置 clash-verge.yaml（Verge 未运行或路径不同）".to_string());
    }

    d
}

fn extract_key(raw: &str, key: &str) -> Option<String> {
    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix(':') {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

/// 提取某个顶层 YAML 块（如 "tun:" 到下一个顶层键之前）。
fn extract_yaml_block(raw: &str, block: &str) -> Option<String> {
    let mut in_block = false;
    let mut out = String::new();
    for line in raw.lines() {
        if !in_block {
            if line.trim_start().starts_with(block) && line.trim_start().ends_with(':') {
                in_block = true;
            }
            continue;
        }
        // 块内：遇到新的顶层键（无缩进且带冒号）即结束
        if !line.starts_with(' ') && !line.starts_with('\t') && line.contains(':') {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_block {
        Some(out)
    } else {
        None
    }
}

/// 生成规则片段（不写入任何文件）：
/// 将已确认的进程名/路径指向指定代理组（MY-VPS），并置于最终 MATCH 之前。
pub fn generate_rules_fragment(
    proxy_group: &str,
    process_names: &[String],
    process_paths: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# ===== LostCodexGateway 生成片段（请核对后手动导入） =====\n");
    out.push_str("# 仅路由以下已确认的 Codex 进程；未列出的应用不受影响\n");
    for name in process_names {
        out.push_str(&format!("  - PROCESS-NAME,{},{}", name, proxy_group));
        out.push('\n');
    }
    for path in process_paths {
        // PROCESS-PATH 规则用 YAML 双引号
        out.push_str(&format!("  - PROCESS-PATH,\"{}\",{}", path.replace('\\', "/"), proxy_group));
        out.push('\n');
    }
    // 导入位置：粘贴到你原配置「最终 MATCH 行」之前。
    // 不生成 MATCH 规则——其余流量必须继续走你原有的最终 MATCH，
    // 否则浏览器/微信等全部应用都会被导进网关。
    out.push_str("# 导入位置：粘贴到你原配置的最终 MATCH 行之前（其余流量不受影响）\n");
    out.push_str("# ===== 片段结束 =====\n");
    out
}

/// 备份本工具指定的文件（用户确认导入前）；返回备份路径。
pub fn backup_file(path: &str) -> Result<PathBuf, String> {
    let src = PathBuf::from(path);
    if !src.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let bak = src.with_file_name(format!(
        "{}.bak_lcfg_{}",
        src.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        ts
    ));
    std::fs::copy(&src, &bak).map_err(|e| e.to_string())?;
    Ok(bak)
}

/// 恢复本工具的备份：仅当目标当前内容与该备份相同或目标不存在时才恢复；
/// 若用户之后改过目标，返回错误并要求人工选择（不强制覆盖）。
pub fn restore_file(path: &str, backup: &str) -> Result<(), String> {
    let src = PathBuf::from(backup);
    if !src.exists() {
        return Err(format!("备份不存在: {}", backup));
    }
    let dst = PathBuf::from(path);
    if dst.exists() {
        let cur = std::fs::read_to_string(&dst).map_err(|e| e.to_string())?;
        let bak_content = std::fs::read_to_string(&src).map_err(|e| e.to_string())?;
        // 简单冲突检测：当前内容与备份不一致时不覆盖（用户可能已改动）
        // 注：本工具只追加过片段；完整 diff 语义见 docs/troubleshooting.md
        if cur != bak_content {
            return Err(
                "目标文件在备份后被修改过：拒绝自动回滚，请人工比对（绝不覆盖用户改动）".to_string(),
            );
        }
    }
    std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_key_basic() {
        let raw = "mixed-port: 2080\nexternal-controller: 127.0.0.1:9097\nmode: rule\n";
        assert_eq!(extract_key(raw, "mixed-port").as_deref(), Some("2080"));
        assert_eq!(extract_key(raw, "external-controller").as_deref(), Some("127.0.0.1:9097"));
        assert_eq!(extract_key(raw, "mode").as_deref(), Some("rule"));
        assert_eq!(extract_key(raw, "absent"), None);
    }

    #[test]
    fn extract_tun_block() {
        let raw = "mode: rule\nipv6: true\ntun:\n  enable: true\n  stack: mixed\ndns:\n  enable: true\n";
        let block = extract_yaml_block(raw, "tun:").unwrap();
        assert!(block.contains("enable: true"));
        assert!(!block.contains("dns:"));
    }

    /// 注册表 DisplayIcon 解析：必须剥掉 `,0` / `,-1` 图标索引与引号，
    /// 且两条顺序都不能弄反（先剥索引再去引号），否则会残留尾部引号。
    /// 修复前这里是硬编码 `D:\Clash Verge\clash-verge.exe`。
    #[test]
    fn display_icon_value_is_parsed_without_hardcoded_drive() {
        for (line, expected) in [
            (r"    DisplayIcon    REG_SZ    D:\Clash Verge\clash-verge.exe,0", r"D:\Clash Verge\clash-verge.exe"),
            (r#"    DisplayIcon    REG_SZ    "D:\Tools\Clash Verge\clash-verge.exe",-1"#, r"D:\Tools\Clash Verge\clash-verge.exe"),
            (r"    DisplayIcon    REG_SZ    C:\verge.exe", r"C:\verge.exe"),
            (r#"    DisplayIcon    REG_SZ    "E:\a b\clash-verge.exe""#, r"E:\a b\clash-verge.exe"),
        ] {
            let idx = line.find("REG_SZ").unwrap();
            let raw = line[idx + "REG_SZ".len()..].trim();
            let path = strip_icon_index(raw);
            assert_eq!(path.trim_matches('"').trim(), expected, "解析 {line:?} 失败");
        }
    }

    /// 常见安装路径必须由环境变量/盘符动态推导，不得出现写死路径。
    #[test]
    fn common_install_paths_are_derived_not_hardcoded() {
        let paths = common_install_paths();
        assert!(!paths.is_empty(), "应至少推导出若干候选路径");
        // 每个候选都必须是绝对路径且以 verge 可执行文件结尾
        for p in &paths {
            assert!(p.is_absolute(), "候选路径应为绝对路径: {:?}", p);
            assert_eq!(p.file_name().and_then(|n| n.to_str()), Some(VERGE_EXE));
        }
    }

    #[test]
    fn fragment_contains_matchname_and_process_rules() {
        let frag = generate_rules_fragment(
            "MY-VPS",
            &["codex.exe".to_string()],
            &[r"C:\path\to\codex.exe".to_string()],
        );
        assert!(frag.contains("PROCESS-NAME,codex.exe,MY-VPS"));
        assert!(frag.contains("PROCESS-PATH,\"C:/path/to/codex.exe\",MY-VPS"));
        // 片段绝不携带 MATCH 规则：全量导流会劫持所有未匹配应用
        assert!(!frag.contains("MATCH,MY-VPS"));
        assert!(!frag.contains("- MATCH"));
    }

    #[test]
    fn fragment_never_routes_generic_processes_unless_listed() {
        let frag = generate_rules_fragment("MY-VPS", &["codex.exe".to_string()], &[]);
        // 不包含通用进程的隐式规则
        assert!(!frag.contains("Code.exe"));
        assert!(!frag.contains("node.exe"));
        assert!(!frag.contains("ssh.exe"));
    }
}
