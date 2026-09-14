//! 托盘（FR-UI-06）：状态图标三色 + 菜单（暂停/恢复、打开设置、退出）。

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri::image::Image;

use dc_bridge::events;
use dc_bridge::state::AppState;
use dc_pipeline::intercept::GuardState;

/// 托盘图标：纯色圆点按状态着色（16×16 程序生成，免图标资源依赖）。
fn dot_image(rgb: [u8; 3]) -> Image<'static> {
    let size = 16u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let r2 = (size / 2 - 1) as f32;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - r2 - 0.5, y as f32 - r2 - 0.5);
            let inside = dx * dx + dy * dy <= r2 * r2;
            let (r, g, b, a) = if inside {
                (rgb[0], rgb[1], rgb[2], 255)
            } else {
                (0, 0, 0, 0)
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    Image::new_owned(rgba, size, size)
}

fn color_for(state: GuardState, found: bool) -> [u8; 3] {
    match (state, found) {
        (GuardState::Active, true) => [34, 197, 94],   // 守护中：绿
        (GuardState::Paused, _) => [245, 158, 11],      // 暂停：橙
        (_, false) => [148, 163, 184],                  // 未发现目标：灰
        _ => [59, 130, 246],                            // 挂起/冷却：蓝
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let state = app.state::<std::sync::Arc<AppState>>();
    let initial = color_for(state.intercept.state(), state.guard_running());

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "pause", "暂停守护", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "open", "打开设置", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(dot_image(initial))
        .tooltip("危信")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "pause" => {
                let state = app.state::<std::sync::Arc<AppState>>();
                let paused = state.intercept.state() == GuardState::Paused;
                state.intercept.set_paused(!paused);
                refresh(app);
                // 状态变更也广播给前端（设置页/托盘同源）
                let _ = app.emit(events::EVENT_STATUS, dc_bridge::commands::status_of(&state));
            }
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button, button_state, .. } = event {
                if button == tauri::tray::MouseButton::Left
                    && button_state == tauri::tray::MouseButtonState::Up
                {
                    let app = tray.app_handle();
                    if let Some(win) = app.get_webview_window("main") {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        } else {
                            show_main(app);
                        }
                    }
                }
            }
        })
        .build(app)?;

    // 状态事件 → 图标刷新（托盘常驻，FR-UI-06）
    let app_for_status = app.clone();
    app.listen_any(events::EVENT_STATUS, move |_event| {
        refresh(&app_for_status);
    });
    Ok(tray)
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn refresh(app: &AppHandle) {
    let state = app.state::<std::sync::Arc<AppState>>();
    let color = color_for(state.intercept.state(), state.guard_running());
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(dot_image(color)));
    }
}
