//! 开机启动（P2）：通过当前用户注册表 Run 键实现，不需要管理员权限。
//!
//! 设计原则（与项目「不静默接管网络」一致）：
//! - **开机启动 ≠ 自动连接隧道**。启用后只是让程序随登录驻留到托盘，
//!   隧道仍需用户显式点击「连接」。这是刻意的默认：静默建立代理出口会让
//!   用户失去对网络出口的知情权。
//! - 只写 `HKCU\...\CurrentVersion\Run`（当前用户），**不碰 HKLM**，
//!   因此不需要提权，也不影响其他用户。
//! - 写入的值只包含可执行文件路径，并带 `--autostart` 标记，便于程序自身
//!   识别本次是「开机自启」而非用户手动双击。
//! - 关闭开关时只删除**本工具自己写入的**同名值；若发现同名值指向其他
//!   程序（异常情况），拒绝删除并如实报告，避免误删他人条目。

use serde::{Deserialize, Serialize};

/// 注册表 Run 键下的值名称。
pub const RUN_VALUE_NAME: &str = "LostCodexGateway";

/// 自启动时附加的命令行标记。
pub const AUTOSTART_FLAG: &str = "--autostart";

/// 开机启动配置状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutostartStatus {
    /// 是否已启用（注册表存在指向本程序的条目）
    pub enabled: bool,
    /// 注册表中当前的命令行值（未启用时为 None）
    pub registered_command: Option<String>,
    /// 本程序当前的可执行文件路径
    pub exe_path: Option<String>,
    /// 注册表条目存在但指向的不是本程序（异常，需用户确认）
    pub points_to_other: bool,
    /// 备注/错误说明
    pub note: String,
}

/// 生成应写入注册表的命令行。
///
/// 路径含空格时用双引号包裹（Windows Run 键的解析约定）。
pub fn build_run_command(exe_path: &str) -> String {
    let trimmed = exe_path.trim();
    if trimmed.contains(' ') {
        format!("\"{}\" {}", trimmed, AUTOSTART_FLAG)
    } else {
        format!("{} {}", trimmed, AUTOSTART_FLAG)
    }
}

/// 判断一个已有的注册表值是否指向本程序（用于安全删除）。
///
/// 规则：去掉首尾引号与参数后，比较路径是否等价（大小写不敏感，
/// 并按 Windows 惯例把 `/` 归一化为 `\`）。
pub fn command_matches_exe(registered: &str, exe_path: &str) -> bool {
    let extracted = extract_exe_from_command(registered);
    let a = normalize_path(&extracted);
    let b = normalize_path(exe_path);
    !a.is_empty() && a == b
}

/// 从注册表命令行中取出可执行文件路径（处理带引号与带参数两种形式）。
pub fn extract_exe_from_command(command: &str) -> String {
    let s = command.trim();
    if let Some(rest) = s.strip_prefix('"') {
        // 引号形式："C:\path with space\app.exe" --flag
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
        return rest.to_string();
    }
    // 无引号形式：取到第一个 ".exe" 为止（路径可能不含空格）
    let lower = s.to_lowercase();
    if let Some(pos) = lower.find(".exe") {
        return s[..pos + 4].to_string();
    }
    s.to_string()
}

fn normalize_path(p: &str) -> String {
    p.trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// 当前可执行文件路径。
pub fn current_exe_path() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

// ---------- Windows 注册表访问 ----------

#[cfg(windows)]
mod win_impl {
    use super::*;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    extern "system" {
        fn RegOpenKeyExW(
            hkey: isize,
            lpsubkey: *const u16,
            uloptions: u32,
            samdesired: u32,
            phkresult: *mut isize,
        ) -> i32;
        fn RegCreateKeyExW(
            hkey: isize,
            lpsubkey: *const u16,
            reserved: u32,
            lpclass: *const u16,
            dwoptions: u32,
            samdesired: u32,
            lpsecurityattributes: *const std::ffi::c_void,
            phkresult: *mut isize,
            lpdwdisposition: *mut u32,
        ) -> i32;
        fn RegQueryValueExW(
            hkey: isize,
            lpvaluename: *const u16,
            lpreserved: *const u32,
            lptype: *mut u32,
            lpdata: *mut u8,
            lpcbdata: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            hkey: isize,
            lpvaluename: *const u16,
            reserved: u32,
            dwtype: u32,
            lpdata: *const u8,
            cbdata: u32,
        ) -> i32;
        fn RegDeleteValueW(hkey: isize, lpvaluename: *const u16) -> i32;
        fn RegCloseKey(hkey: isize) -> i32;
    }

    /// HKEY_CURRENT_USER
    const HKCU: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_SZ: u32 = 1;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_SUCCESS: i32 = 0;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 读取 Run 键中本工具的值。返回 Ok(Some(cmd)) / Ok(None) / Err(错误码)
    pub fn read_run_value() -> Result<Option<String>, i32> {
        unsafe {
            let mut hkey: isize = 0;
            let rc = RegOpenKeyExW(
                HKCU,
                wide(RUN_KEY).as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(rc);
            }
            let mut ty: u32 = 0;
            let mut size: u32 = 0;
            // 先查大小
            let rc = RegQueryValueExW(
                hkey,
                wide(RUN_VALUE_NAME).as_ptr(),
                std::ptr::null(),
                &mut ty,
                std::ptr::null_mut(),
                &mut size,
            );
            if rc == ERROR_FILE_NOT_FOUND {
                RegCloseKey(hkey);
                return Ok(None);
            }
            if rc != ERROR_SUCCESS || size == 0 {
                RegCloseKey(hkey);
                return Err(rc);
            }
            let mut buf = vec![0u8; size as usize];
            let rc = RegQueryValueExW(
                hkey,
                wide(RUN_VALUE_NAME).as_ptr(),
                std::ptr::null(),
                &mut ty,
                buf.as_mut_ptr(),
                &mut size,
            );
            RegCloseKey(hkey);
            if rc != ERROR_SUCCESS {
                return Err(rc);
            }
            // 按 UTF-16 解析（去掉结尾 NUL）
            let u16s: Vec<u16> = buf
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            Ok(Some(String::from_utf16_lossy(&u16s)))
        }
    }

    /// 写入 Run 值（REG_SZ）。
    pub fn write_run_value(command: &str) -> Result<(), i32> {
        unsafe {
            let mut hkey: isize = 0;
            let mut disp: u32 = 0;
            let rc = RegCreateKeyExW(
                HKCU,
                wide(RUN_KEY).as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut hkey,
                &mut disp,
            );
            if rc != ERROR_SUCCESS {
                return Err(rc);
            }
            let data = wide(command);
            let bytes = std::slice::from_raw_parts(
                data.as_ptr() as *const u8,
                data.len() * 2,
            );
            let rc = RegSetValueExW(
                hkey,
                wide(RUN_VALUE_NAME).as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr(),
                bytes.len() as u32,
            );
            RegCloseKey(hkey);
            if rc != ERROR_SUCCESS {
                return Err(rc);
            }
            Ok(())
        }
    }

    /// 删除 Run 值。
    pub fn delete_run_value() -> Result<(), i32> {
        unsafe {
            let mut hkey: isize = 0;
            let rc = RegOpenKeyExW(
                HKCU,
                wide(RUN_KEY).as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(rc);
            }
            let rc = RegDeleteValueW(hkey, wide(RUN_VALUE_NAME).as_ptr());
            RegCloseKey(hkey);
            if rc != ERROR_SUCCESS {
                return Err(rc);
            }
            Ok(())
        }
    }
}

/// 查询开机启动状态（只读）。
pub fn status() -> AutostartStatus {
    let exe = current_exe_path();
    #[cfg(windows)]
    {
        match win_impl::read_run_value() {
            Ok(Some(cmd)) => {
                let points_to_other = match &exe {
                    Some(e) => !command_matches_exe(&cmd, e),
                    None => true,
                };
                AutostartStatus {
                    enabled: !points_to_other,
                    registered_command: Some(cmd.clone()),
                    exe_path: exe,
                    points_to_other,
                    note: if points_to_other {
                        format!(
                            "注册表条目「{}」指向其他程序：{}。为安全起见不会自动删除，请自行确认。",
                            RUN_VALUE_NAME, cmd
                        )
                    } else {
                        "已启用：登录后驻留到系统托盘（不会自动连接隧道）".to_string()
                    },
                }
            }
            Ok(None) => AutostartStatus {
                enabled: false,
                registered_command: None,
                exe_path: exe,
                points_to_other: false,
                note: "未启用".to_string(),
            },
            Err(code) => AutostartStatus {
                enabled: false,
                registered_command: None,
                exe_path: exe,
                points_to_other: false,
                note: format!("读取注册表失败（错误码 {}）", code),
            },
        }
    }
    #[cfg(not(windows))]
    {
        AutostartStatus {
            enabled: false,
            registered_command: None,
            exe_path: exe,
            points_to_other: false,
            note: "仅 Windows 支持".to_string(),
        }
    }
}

/// 启用开机启动。返回写入的命令行。
pub fn enable() -> Result<String, String> {
    let exe = current_exe_path().ok_or_else(|| "无法确定当前程序路径".to_string())?;
    let cmd = build_run_command(&exe);
    #[cfg(windows)]
    {
        win_impl::write_run_value(&cmd)
            .map_err(|c| format!("写入注册表失败（错误码 {}）", c))?;
        Ok(cmd)
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
        Err("仅 Windows 支持开机启动".to_string())
    }
}

/// 关闭开机启动。
///
/// 安全约束：若注册表条目指向的不是本程序，拒绝删除（避免误删他人条目）。
pub fn disable() -> Result<String, String> {
    #[cfg(windows)]
    {
        let exe = current_exe_path().ok_or_else(|| "无法确定当前程序路径".to_string())?;
        match win_impl::read_run_value() {
            Ok(Some(cmd)) => {
                if !command_matches_exe(&cmd, &exe) {
                    return Err(format!(
                        "注册表条目指向其他程序（{}），拒绝自动删除以免误删他人配置",
                        cmd
                    ));
                }
                win_impl::delete_run_value()
                    .map_err(|c| format!("删除注册表值失败（错误码 {}）", c))?;
                Ok("已关闭开机启动".to_string())
            }
            Ok(None) => Ok("开机启动本就未启用".to_string()),
            Err(c) => Err(format!("读取注册表失败（错误码 {}）", c)),
        }
    }
    #[cfg(not(windows))]
    {
        Err("仅 Windows 支持开机启动".to_string())
    }
}

/// 判断本次进程是否由开机自启拉起（供 UI 提示 / 决定是否直接隐藏到托盘）。
pub fn launched_by_autostart() -> bool {
    std::env::args().any(|a| a == AUTOSTART_FLAG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_quotes_paths_with_spaces() {
        let c = build_run_command(r"C:\Program Files\LostCodexGateway\lostcodexgateway.exe");
        assert!(c.starts_with('"'), "含空格的路径必须加引号: {}", c);
        assert!(c.ends_with(AUTOSTART_FLAG));
        // 不含空格的路径不加引号
        let c2 = build_run_command(r"C:\app.exe");
        assert_eq!(c2, r"C:\app.exe --autostart");
    }

    #[test]
    fn extract_exe_handles_quoted_and_bare_forms() {
        assert_eq!(
            extract_exe_from_command(r#""C:\Program Files\App\app.exe" --autostart"#),
            r"C:\Program Files\App\app.exe"
        );
        assert_eq!(
            extract_exe_from_command(r"C:\App\app.exe --autostart"),
            r"C:\App\app.exe"
        );
        assert_eq!(extract_exe_from_command(r"C:\App\app.exe"), r"C:\App\app.exe");
    }

    #[test]
    fn command_matching_is_case_and_slash_insensitive() {
        let reg = r#""C:\Program Files\LCFG\lcfg.exe" --autostart"#;
        assert!(command_matches_exe(reg, r"c:\program files\lcfg\lcfg.exe"));
        assert!(command_matches_exe(reg, r"C:/Program Files/LCFG/lcfg.exe"));
        // 指向别的程序必须判为不匹配（用于拒绝误删）
        assert!(!command_matches_exe(reg, r"C:\Other\other.exe"));
    }

    #[test]
    fn empty_registered_value_never_matches() {
        // 空值不得被判定为「指向本程序」，否则 disable 会误删
        assert!(!command_matches_exe("", r"C:\App\app.exe"));
        assert!(!command_matches_exe("   ", r"C:\App\app.exe"));
    }

    #[test]
    fn autostart_flag_is_detectable() {
        // 标记常量必须与 build_run_command 使用的一致
        let c = build_run_command(r"C:\App\app.exe");
        assert!(c.contains(AUTOSTART_FLAG));
    }
}
