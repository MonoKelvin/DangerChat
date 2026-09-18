//! 托盘（FR-UI-06）：品牌 logo 按状态动态调整色相（不生成多张图）+ 菜单。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager};

use dc_bridge::events;
use dc_bridge::state::AppState;
use dc_pipeline::intercept::GuardState;

/// 全幅拉伸后的品牌 logo（32px，托盘基础图；状态变色在内存中完成）
static LOGO: &[u8] = include_bytes!("../icons/logo-32.png");

/// logo 基础色相（暖红 ≈11°）；主题色切换时由前端经 set_tray_hue 覆盖
const LOGO_HUE: f32 = 11.0;

/// ── 色相调整算法（RGB ↔ HSL，保留 alpha）──
fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    (h, s, l)
}

fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    let mut t = t;
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s < 1e-6 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let t = h / 360.0;
    (
        hue_to_rgb(p, q, t + 1.0 / 3.0),
        hue_to_rgb(p, q, t),
        hue_to_rgb(p, q, t - 1.0 / 3.0),
    )
}

/// 就地色相旋转 + 饱和度/亮度调整（灰色像素不动，alpha 保留）。
fn tint(rgba: &mut [u8], hue_shift: f32, sat_scale: f32, l_add: f32) {
    // as_chunks_mut 比 chunks_exact_mut 少一次边界检查，且尾部残块语义明确（此处丢弃）
    for px in rgba.as_chunks_mut::<4>().0 {
        let (r, g, b) = (
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
        );
        let (h, s, l) = rgb_to_hsl(r, g, b);
        if s < 1e-4 {
            // 无彩色：亮度调整仍生效（浅灰/深灰态）
            let g2 = (l + l_add).clamp(0.0, 1.0);
            px[0] = (g2 * 255.0).round() as u8;
            px[1] = (g2 * 255.0).round() as u8;
            px[2] = (g2 * 255.0).round() as u8;
            continue;
        }
        let (r, g, b) = hsl_to_rgb(
            (h + hue_shift).rem_euclid(360.0),
            (s * sat_scale).min(1.0),
            (l + l_add).clamp(0.0, 1.0),
        );
        px[0] = (r * 255.0).round() as u8;
        px[1] = (g * 255.0).round() as u8;
        px[2] = (b * 255.0).round() as u8;
    }
}

/// 状态 → (饱和度, 亮度)：守护中 = 主题色；挂起/冷却 = 浅灰；暂停(禁用) = 深灰；
/// 未发现目标 = 去饱和。色相偏移由调用方按主题色相换算。
fn tint_for(state: GuardState, found: bool) -> (f32, f32) {
    match (state, found) {
        (GuardState::Active, true) => (1.0, 0.0),
        (GuardState::Paused, _) => (0.06, -0.18),
        (GuardState::Suspended, _) | (GuardState::Cooldown, _) => (0.1, 0.22),
        (_, false) => (0.12, 0.0),
    }
}

/// 按状态 + 主题色相变色的托盘图标（同一张 logo，内存中调整）
fn logo_image(state: &AppState) -> Image<'static> {
    let base_hue = state
        .tray_hue_deg
        .load(std::sync::atomic::Ordering::Relaxed) as f32;
    let img = image::load_from_memory(LOGO)
        .expect("内嵌 logo 解码失败")
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    let mut rgba = img.into_raw();
    let (sat, l) = tint_for(state.intercept.state(), state.guard_running());
    tint(&mut rgba, base_hue - LOGO_HUE, sat, l);
    Image::new_owned(rgba, w, h)
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let state = app.state::<std::sync::Arc<AppState>>();
    let initial = logo_image(&state);

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "打开设置", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "pause", "暂停守护", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(initial)
        // 名称唯一来源：src-tauri/tauri.conf.json 的 productName
        .tooltip(
            app.config()
                .product_name
                .clone()
                .unwrap_or_else(|| "DangerChat".into()),
        )
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
            "quit" => {
                crate::shutdown_and_exit(app);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                if button == tauri::tray::MouseButton::Left
                    && button_state == tauri::tray::MouseButtonState::Up
                {
                    let app = tray.app_handle();
                    if let Some(win) = app.get_webview_window("main") {
                        // 「已显示」= 可见**且未最小化**：Windows 下最小化窗口仍带 WS_VISIBLE，
                        // 只看 is_visible 会把「点了托盘让它出来」判成「已显示 → 隐藏」，
                        // 用户看到的是窗口（任务栏按钮）反而消失。
                        let shown = win.is_visible().unwrap_or(false)
                            && !win.is_minimized().unwrap_or(false);
                        if shown {
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

/// 唤出主窗口（托盘左键、托盘菜单「打开设置」两处共用）。
///
/// 注意：**不要在单实例回调里直接调本函数** —— 那个回调运行在跨进程消息
/// 投递的窗口过程内（同步等待返回的上下文），此处 `show`/`set_focus`
/// 触发的操作可能反向阻塞发送方，构成经典死锁。单实例回调请用
/// `show_main_async`。
pub fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        // 必须先 unminimize（SW_RESTORE）再 show：窗口处于最小化态时，
        // show()（SW_SHOW）只是「显示为最小化」，任务栏留个按钮而窗口依旧不出现。
        // 托盘左键在 hide/show 间切换，用户很容易走到「最小化 → 点托盘」这条路径上。
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 异步唤出主窗口：把操作投递到 Tauri 事件循环，不在调用者线程同步执行。
///
/// 专供单实例回调（运行在跨进程消息投递的窗口过程内）使用，避免跨进程死锁。
pub fn show_main_async(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        show_main(&app);
    });
}

pub fn refresh(app: &AppHandle) {
    let state = app.state::<std::sync::Arc<AppState>>();
    let icon = logo_image(&state);
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(icon));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 色相旋转：暖红 → 蓝，红色分量显著下降、蓝色上升
    #[test]
    fn hue_shift_changes_dominant_channel() {
        let mut rgba = [191u8, 64, 42, 255];
        tint(&mut rgba, 215.0 - LOGO_HUE, 1.0, 0.0);
        assert!(rgba[0] < rgba[2], "红({}) 应低于 蓝({})", rgba[0], rgba[2]);
        assert_eq!(rgba[3], 255, "alpha 保留");
    }

    /// 饱和度缩放 + 亮度提升 → 浅灰（挂起态）
    #[test]
    fn tint_light_gray() {
        let mut rgba = [191u8, 64, 42, 255];
        tint(&mut rgba, 0.0, 0.1, 0.22);
        let (r, g, b) = (rgba[0] as u32, rgba[1] as u32, rgba[2] as u32);
        assert!(
            r.abs_diff(g) < 24 && g.abs_diff(b) < 24,
            "应接近灰色：{rgba:?}"
        );
        assert!(rgba[0] > 150, "亮度应明显提升：{rgba:?}");
    }

    /// 饱和度压低 + 亮度下降 → 深灰（禁用态）
    #[test]
    fn tint_dark_gray() {
        let mut rgba = [191u8, 64, 42, 255];
        tint(&mut rgba, 0.0, 0.06, -0.18);
        let (r, g, b) = (rgba[0] as u32, rgba[1] as u32, rgba[2] as u32);
        assert!(
            r.abs_diff(g) < 24 && g.abs_diff(b) < 24,
            "应接近灰色：{rgba:?}"
        );
        assert!(rgba[0] < 130, "亮度应明显下降：{rgba:?}");
    }

    /// 无彩色像素：只受亮度影响，不产生色相
    #[test]
    fn achromatic_only_lightness() {
        let mut rgba = [128u8, 128, 128, 255];
        tint(&mut rgba, 180.0, 1.0, 0.2);
        assert_eq!(rgba, [179, 179, 179, 255]);
    }
}
