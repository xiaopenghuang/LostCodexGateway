//! 子进程启动辅助。
//!
//! LostCodexGateway 是 GUI 程序：后台调用 ssh-keygen / ssh-keyscan /
//! tasklist / powershell / ssh 等命令行工具时，Windows 默认会为每个
//! 控制台子进程创建一个可见的控制台窗口（表现为点击功能时闪现黑框）。
//! 这里统一加上 CREATE_NO_WINDOW 抑制。
//!
//! 唯一例外是「应用」页启动 Codex CLI 的交互终端（launchers 中显式
//! 使用 CREATE_NEW_CONSOLE，因为用户需要在该窗口中与 CLI 交互）。

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// std::process::Command，启动时不创建控制台窗口。
pub fn std_cmd(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// tokio::process::Command，启动时不创建控制台窗口。
pub fn tokio_cmd(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
