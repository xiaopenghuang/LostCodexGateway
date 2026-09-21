//! 单实例守卫（对应风险清单 R10）。
//!
//! 问题：开机自启与用户手动双击可能同时发生（例如用户不知道程序已在托盘里，
//! 又点了一次图标）。若无守卫，会起两个独立进程，各自管理一条 SSH 隧道、
//! 各自绑定同一个本地 SOCKS 端口——后启动的那个会因端口占用而失败，但两个
//! 进程的状态机互不知情，托盘会出现两个图标，退出行为也不一致。
//!
//! 方案：Windows 命名互斥体（`CreateMutexW`）。进程启动时尝试创建名为
//! `Local\LostCodexGateway.SingleInstance` 的互斥体：
//! - 创建成功且 `GetLastError() == ERROR_ALREADY_EXISTS` → 已有实例在跑。
//! - 用 `Local\` 前缀（而非 `Global\`）把作用域限定在当前登录会话，
//!   这样多用户/远程桌面各自可以有一份，符合「只写当前用户注册表」的一致性。
//!
//! 不引入第三方 crate：本模块只用 Win32 API，与项目「依赖尽量少」的取向一致。
//!
//! 互斥体句柄在进程存活期间必须保持有效（不能 drop/CloseHandle），
//! 否则守卫会失效——因此由 `SingleInstanceGuard` 持有，并交给 Tauri 的
//! managed state 保管到进程结束。

/// 互斥体名称。`Local\` 前缀 = 当前登录会话作用域（非 Global）。
pub const MUTEX_NAME: &str = r"Local\LostCodexGateway.SingleInstance";

/// 「请把主窗口显示出来」信号的事件名。
///
/// 第二个实例被守卫拦下时，不应静默消失——用户刚点了图标，必须看到反馈。
/// 做法：第二个实例 SetEvent 一个命名事件，已在运行的实例有一个等待线程
/// 收到后唤出主窗口。这比「第二个实例直接退出」体验好得多。
pub const SHOW_EVENT_NAME: &str = r"Local\LostCodexGateway.ShowWindow";

// ---------- Win32 声明 ----------

#[cfg(windows)]
extern "system" {
    fn CreateMutexW(
        lpMutexAttributes: *const std::ffi::c_void,
        bInitialOwner: i32,
        lpName: *const u16,
    ) -> isize;
    fn CreateEventW(
        lpEventAttributes: *const std::ffi::c_void,
        bManualReset: i32,
        bInitialState: i32,
        lpName: *const u16,
    ) -> isize;
    fn SetEvent(hEvent: isize) -> i32;
    fn WaitForSingleObject(hHandle: isize, dwMilliseconds: u32) -> u32;
    fn GetLastError() -> u32;
    fn CloseHandle(hObject: isize) -> i32;
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 请求「唤出主窗口」：由被拦下的第二个实例调用。
pub fn signal_show_window() {
    #[cfg(windows)]
    {
        unsafe {
            // 自动重置事件（bManualReset=0）；即使第一个实例尚未建好事件，
            // CreateEventW 也会创建它，后续 WaitForSingleObject 即可收到。
            let h = CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT_NAME).as_ptr());
            if h != 0 {
                SetEvent(h);
                CloseHandle(h);
            }
        }
    }
}

/// 在已运行实例中等待「唤出窗口」信号的后台线程。
///
/// 用一个短周期的轮询式等待而非永久阻塞：`WaitForSingleObject` 会在进程
/// 退出时随主线程结束而中断，但轮询让线程在收到 shutdown 标志时能主动退出，
/// 避免在 `app.exit()` 拆窗口的过程中还去调用 `show()` 造成竞争。
pub fn spawn_show_window_listener<F>(on_signal: F) -> std::sync::Arc<std::sync::atomic::AtomicBool>
where
    F: Fn() + Send + 'static,
{
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

    #[cfg(windows)]
    {
        let running_thread = running.clone();
        std::thread::spawn(move || unsafe {
            use std::sync::atomic::Ordering;
            let h = CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT_NAME).as_ptr());
            if h == 0 {
                return;
            }
            while running_thread.load(Ordering::Relaxed) {
                // 500ms 超时轮询，便于及时响应 shutdown 标志
                let rc = WaitForSingleObject(h, 500);
                const WAIT_OBJECT_0: u32 = 0;
                if rc == WAIT_OBJECT_0 && running_thread.load(Ordering::Relaxed) {
                    on_signal();
                }
            }
            CloseHandle(h);
        });
    }
    #[cfg(not(windows))]
    {
        let _ = on_signal;
    }

    running
}

/// 持有命名互斥体的守卫。
///
/// **不要提前 drop 本结构体**：一旦句柄被关闭，另一个实例就能成功创建同名
/// 互斥体，守卫随即失效。应当把它放进 Tauri managed state（App 生命周期内
/// 始终存活）。
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: isize,
}

/// 尝试成为唯一实例（吞掉所有权）。
///
/// 返回 `Some(guard)` 表示本进程是唯一实例，守卫需被长期持有；
/// 返回 `None` 表示已有实例在运行，调用方应唤醒旧窗口并立即退出。
pub fn acquire() -> Option<SingleInstanceGuard> {
    #[cfg(windows)]
    {
        const ERROR_ALREADY_EXISTS: u32 = 183;
        let name = wide(MUTEX_NAME);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle == 0 {
            // 创建失败（极端情况）：不阻塞用户，放行本次启动。
            // 宁可允许极端情况下多开，也不要把用户挡在程序之外。
            return Some(SingleInstanceGuard { handle: 0 });
        }
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already {
            // 已有实例：关闭本次句柄，报告非唯一
            unsafe {
                CloseHandle(handle);
            }
            return None;
        }
        return Some(SingleInstanceGuard { handle });
    }
    #[cfg(not(windows))]
    {
        Some(SingleInstanceGuard {})
    }
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutex_name_uses_local_scope_not_global() {
        // 必须是当前会话作用域（Local\），不能是 Global\：
        // Global 会让多用户/远程桌面互相排斥，与本工具「仅当前用户」的定位不符。
        assert!(MUTEX_NAME.starts_with(r"Local\"), "应使用 Local\\ 前缀: {}", MUTEX_NAME);
        assert!(!MUTEX_NAME.starts_with(r"Global\"));
        // 名称需带产品标识，避免与其他程序撞名
        assert!(MUTEX_NAME.contains("LostCodexGateway"));
    }

    #[cfg(windows)]
    #[test]
    fn acquire_returns_some_when_no_other_instance() {
        // 首次获取应成功（测试进程内只获取一次）
        let guard = acquire();
        assert!(guard.is_some(), "无其他实例时应能取得守卫");
        // 守卫存活期间，同进程再次尝试应报告「已存在」
        let second = acquire();
        assert!(second.is_none(), "守卫未释放时第二次获取必须失败");
        drop(guard);
        // 释放后应能重新获取
        let third = acquire();
        assert!(third.is_some(), "释放守卫后应能重新获取");
    }
}
