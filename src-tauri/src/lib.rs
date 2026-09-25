//! LostCodexGateway 库入口：Tauri 应用组装。

pub mod bridge;
pub mod autostart;
pub mod commands;
pub mod config;
pub mod diagnostics;
pub mod launchers;
pub mod mihomo;
pub mod preflight;
pub mod procutil;
pub mod single_instance;
pub mod ssh;
pub mod state;
pub mod verify;
pub mod wsl;

use state::GatewayStateMachine;
use tauri::Manager;

/// 托盘图标持有状态：TrayIcon 必须被持有才不会被释放。
/// None = 托盘创建失败，此时关闭按钮保持「直接退出」行为（防止窗口被困住无法退出）。
struct TrayState(parking_lot::Mutex<Option<tauri::tray::TrayIcon>>);

/// 单实例守卫的持有者（见 `single_instance` 模块）。
///
/// 互斥体句柄必须在整个进程生命周期内保持有效，否则守卫失效。
/// 放进 Tauri managed state 就是为了让它在 App 存活期间不被 drop。
struct InstanceGuardState(#[allow(dead_code)] single_instance::SingleInstanceGuard);

/// 唤出窗口监听线程的开关。窗口销毁时置 false，让后台线程干净退出，
/// 避免它在 Tauri 拆除窗口的过程中还去调用 `show()`。
struct ShowListenerState(std::sync::Arc<std::sync::atomic::AtomicBool>);

/// 托盘菜单/图标事件处理（显示主界面）。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ---- 单实例守卫（R10）----
    // 必须在任何窗口/托盘创建之前判断：开机自启与用户手动双击可能撞车。
    // 已有实例在跑时，本进程只负责「让对方把窗口显示出来」，然后立刻退出，
    // 绝不去动隧道状态——否则两个进程会争抢同一个本地端口。
    let guard = match single_instance::acquire() {
        Some(g) => g,
        None => {
            // 唤出已在运行的实例主窗口，让用户看到「程序已经在托盘里了」。
            single_instance::signal_show_window();
            return;
        }
    };

    let config = config::load().unwrap_or_default();
    tauri::Builder::default()
        .manage(GatewayStateMachine::new(config))
        .manage(InstanceGuardState(guard))
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
            let show = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let tray_result = TrayIconBuilder::with_id("lcfg-tray")
                .tooltip("LostCodexGateway — 隧道保持运行")
                .icon(
                    app.default_window_icon()
                        .expect("缺少默认窗口图标")
                        .clone(),
                )
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => {
                        // 先干净断开隧道（只清理本工具创建的 ssh），再退出。
                        // 断开流程有内部超时上限，异常时也不会把用户困在无法退出的状态。
                        let app = app.clone();
                        let inner = app.state::<GatewayStateMachine>().inner.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = crate::commands::disconnect_impl(&app, &inner).await;
                            app.exit(0);
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app);
            match tray_result {
                Ok(tray) => {
                    app.manage(TrayState(parking_lot::Mutex::new(Some(tray))));
                }
                Err(e) => {
                    eprintln!("托盘创建失败（关闭按钮将直接退出而非隐藏）: {e}");
                    app.manage(TrayState(parking_lot::Mutex::new(None)));
                }
            }
            // 监听「唤出窗口」信号：另一个实例被守卫拦下时会 SetEvent 这个事件。
            // 没有它的话，用户双击图标会「什么都没发生」，看起来像程序坏了。
            let handle = app.handle().clone();
            let listener_running = single_instance::spawn_show_window_listener(move || {
                show_main_window(&handle);
            });
            app.manage(ShowListenerState(listener_running));

            // 开机自启时直接驻留托盘，不弹主窗口——但**不自动连接隧道**：
            // 静默建立代理出口会让用户失去对网络出口的知情权。用户需自行点「连接」。
            if autostart::launched_by_autostart() {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }

            // 清理上次遗留的 ssh 隧道（强杀 / 崩溃 / 覆盖安装会留下孤儿）。
            // 放 setup 末尾：不阻塞窗口显示，扫描本身在阻塞线程池里跑。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    commands::cleanup_stale_tunnels(&handle).await;
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 点关闭按钮 = 隐藏到托盘（隧道保持运行），不退出应用。
            // 真正退出走托盘菜单「退出」（先断开隧道）。
            // 托盘不可用（None）时不拦截，回退为直接退出。
            if window.label() == "main" {
                match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        let has_tray = window
                            .state::<TrayState>()
                            .0
                            .lock()
                            .is_some();
                        if has_tray {
                            api.prevent_close();
                            let _ = window.hide();
                        }
                    }
                    tauri::WindowEvent::Destroyed => {
                        // 真正退出：通知唤出监听线程结束，避免它在拆窗口时抢着 show()。
                        window
                            .state::<ShowListenerState>()
                            .0
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::detect_ssh_env,
            commands::save_server,
            commands::delete_server,
            commands::switch_server,
            commands::test_servers,
            commands::save_settings,
            commands::fetch_host_key,
            commands::confirm_host_key,
            commands::test_connection,
            commands::connect,
            commands::disconnect,
            commands::get_launch_preview,
            commands::launch_codex_cli,
            commands::detect_mihomo,
            commands::generate_mihomo_fragment,
            commands::backup_mihomo_file,
            commands::restore_mihomo_file,
            commands::run_diagnostics,
            commands::get_last_diagnostics,
            commands::diagnose_client,
            commands::set_expected_egress_ip,
            commands::export_diagnostics,
            commands::detect_wsl,
            commands::get_wsl_proxy_command,
            commands::get_autostart_status,
            commands::set_autostart,
            commands::run_preflight,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LostCodexGateway");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;
    use state::GatewayState as GS;

    #[test]
    fn initial_state_unconfigured() {
        let m = GatewayStateMachine::new(GatewayConfig::default());
        assert_eq!(m.snapshot().state, GS::Unconfigured);
    }

    #[test]
    fn initial_state_ready_when_configured() {
        let mut cfg = GatewayConfig::default();
        {
            let s = cfg.active_server_mut().expect("默认配置应含一台服务器");
            s.host = "vps.example.com".into();
            s.username = "ubuntu".into();
        }
        let m = GatewayStateMachine::new(cfg);
        assert_eq!(m.snapshot().state, GS::Ready);
    }

    #[test]
    fn state_transition_and_log() {
        let m = GatewayStateMachine::new(GatewayConfig::default());
        m.set_state(GS::Connecting);
        assert_eq!(m.snapshot().state, GS::Connecting);
        m.log("info", "test", "hello");
        let snap = m.snapshot();
        assert_eq!(snap.recent_logs.len(), 1);
        assert_eq!(snap.recent_logs[0].component, "test");
    }
}
