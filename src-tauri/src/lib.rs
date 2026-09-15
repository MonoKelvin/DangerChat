//! 危信主程序壳：装配（bootstrap）+ 命令注册 + 托盘 + 泵/监视线程 + 静默启动。

mod bootstrap;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{Manager, RunEvent};

/// 退出意图：托盘"退出"设置为 true，允许进程真正退出；窗口关闭不设置，保持托盘常驻。
static ALLOW_EXIT: AtomicBool = AtomicBool::new(false);

pub fn set_exit_intent() {
    ALLOW_EXIT.store(true, Ordering::SeqCst);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            dc_bridge::commands::get_config_schema,
            dc_bridge::commands::get_config,
            dc_bridge::commands::set_config,
            dc_bridge::commands::list_contacts,
            dc_bridge::commands::set_contact_profile,
            dc_bridge::commands::get_rules,
            dc_bridge::commands::save_rules,
            dc_bridge::commands::import_model,
            dc_bridge::commands::pause_guard,
            dc_bridge::commands::resume_guard,
            dc_bridge::commands::get_guard_status,
            dc_bridge::commands::clear_logs,
            dc_bridge::commands::alert_action,
        ])
        .setup(|app| {
            // 1) 装配（日志/配置/钩子/发现前置）
            let state = bootstrap::bootstrap(app.handle());
            app.manage(Arc::clone(&state));

            // 2) 托盘（常驻，FR-UI-06）
            tray::build(app.handle())?;

            // 3) 泵线程（AlertBus → 事件/弹窗）+ 窗口发现（spawn Guard）
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("bridge-pump-main".into())
                .spawn(move || dc_bridge::pump::spawn(handle))
                .map_err(|e| format!("pump 启动失败：{e}"))?;
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("window-watch-main".into())
                .spawn(move || dc_bridge::window_watch::spawn(handle))
                .map_err(|e| format!("window-watch 启动失败：{e}"))?;

            // 4) 静默启动（FR-UI-07）：silent_start（默认 true）或 --silent 参数
            let silent = state
                .config
                .snapshot()
                .bool_or("general.silent_start", true)
                || std::env::args().any(|a| a == "--silent");
            if !silent {
                if let Some(win) = app.get_webview_window("main") {
                    win.show()?;
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("tauri 构建失败")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                // 只有托盘明确退出时允许真正退出；窗口关闭保持托盘常驻
                if !ALLOW_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
