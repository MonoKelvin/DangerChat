//! 托盘（FR-UI-06）：品牌 logo 按状态动态调整色相（不生成多张图）+ 菜单。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager};

/// 暂停/开启守护菜单项按状态切换的文案（点了「暂停守护」后应显示「开启守护」，反之亦然）。
fn pause_label(paused: bool) -> &'static str {
    if paused {
        "开启守护"
    } else {
        "暂停守护"
    }
}

use dc_bridge::events;
use dc_bridge::state::AppState;
use dc_pipeline::intercept::GuardState;

/// 全幅拉伸后的品牌 logo（32px，托盘基础图；状态变色在内存中完成）
static LOGO: &[u8] = include_bytes!("../icons/logo-32.png");

/// 「暂停/开启守护」菜单项句柄（由 build 存入 state，refresh 时按守护态更新文案）。
struct PauseMenuItem(MenuItem<tauri::Wry>);

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

/// 就地着色（灰色像素不动，alpha 保留）：彩色像素 H/S **直接设成主题色**，
/// 只保留自身亮度 L 维持明暗层次；`l_mul` 整体压暗（<1 变暗，=1 本色）。
///
/// 为何压暗而非降饱和：旧做法挂起态降饱和 → 图标接近白/灰，与浅色托盘背景难分辨（用户反馈）。
fn tint(rgba: &mut [u8], accent_hue: f32, accent_sat: f32, l_mul: f32) {
    // as_chunks_mut 比 chunks_exact_mut 少一次边界检查，且尾部残块语义明确（此处丢弃）
    for px in rgba.as_chunks_mut::<4>().0 {
        let (r, g, b) = (
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
        );
        let (_, s, l) = rgb_to_hsl(r, g, b);
        if s < 1e-4 {
            // 无彩色（黑白灰描边）：保持中性，只随 l_mul 压暗
            let g2 = (l * l_mul).clamp(0.0, 1.0);
            px[0] = (g2 * 255.0).round() as u8;
            px[1] = (g2 * 255.0).round() as u8;
            px[2] = (g2 * 255.0).round() as u8;
            continue;
        }
        let (r, g, b) = hsl_to_rgb(accent_hue, accent_sat, (l * l_mul).clamp(0.0, 1.0));
        px[0] = (r * 255.0).round() as u8;
        px[1] = (g * 255.0).round() as u8;
        px[2] = (b * 255.0).round() as u8;
    }
}

/// 状态 → 亮度系数 `l_mul`：守护中 = 本色(1.0)；等待目标 = 略暗；
/// 挂起/冷却 = 压暗；暂停(禁用) = 最暗。彩色像素的 H/S 恒为主题色，仅明暗区分状态。
fn tint_for(state: GuardState, found: bool) -> f32 {
    match (state, found) {
        (GuardState::Active, true) => 1.0,
        (GuardState::Active, false) => 0.8,
        (GuardState::Paused, _) => 0.45,
        (GuardState::Suspended, _) | (GuardState::Cooldown, _) => 0.6,
    }
}

/// 按状态 + 主题色着色的托盘图标（同一张 logo，内存中调整）
fn logo_image(state: &AppState) -> Image<'static> {
    let accent_hue = state
        .tray_hue_deg
        .load(std::sync::atomic::Ordering::Relaxed) as f32;
    let accent_sat = state
        .tray_sat_pct
        .load(std::sync::atomic::Ordering::Relaxed) as f32
        / 100.0;
    let img = image::load_from_memory(LOGO)
        .expect("内嵌 logo 解码失败")
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    let mut rgba = img.into_raw();
    let l_mul = tint_for(state.intercept.state(), state.guard_running());
    tint(&mut rgba, accent_hue, accent_sat, l_mul);
    Image::new_owned(rgba, w, h)
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let state = app.state::<std::sync::Arc<AppState>>();
    let initial = logo_image(&state);

    let paused = state.intercept.state() == GuardState::Paused;
    let pause_item = MenuItem::with_id(app, "pause", pause_label(paused), true, None::<&str>)?;
    // 存一份句柄，供 refresh 按守护状态切换文案（「暂停守护」↔「开启守护」）
    app.manage(PauseMenuItem(pause_item.clone()));
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "打开设置", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &pause_item,
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
    // 菜单文案随守护态切换：暂停后显示「开启守护」，恢复后显示「暂停守护」
    let paused = state.intercept.state() == GuardState::Paused;
    if let Some(item) = app.try_state::<PauseMenuItem>() {
        let _ = item.0.set_text(pause_label(paused));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 彩色像素直接采用主题色相：暖红原图 → 蓝色主题（H≈220），蓝分量应主导
    #[test]
    fn colored_pixel_takes_accent_hue() {
        let mut rgba = [191u8, 64, 42, 255];
        tint(&mut rgba, 220.0, 0.8, 1.0);
        assert!(rgba[2] > rgba[0], "蓝({}) 应高于 红({})", rgba[2], rgba[0]);
        assert_eq!(rgba[3], 255, "alpha 保留");
    }

    /// 彩色像素采用主题色饱和度：低饱和主题（S=0.2）→ 结果饱和度低（RGB 三通道差小）
    #[test]
    fn colored_pixel_takes_accent_sat() {
        // 高饱和红原图，着色成低饱和主题色（H=200 S=0.2 L 保留）
        let mut high = [220u8, 30, 30, 255];
        tint(&mut high, 200.0, 0.2, 1.0);
        let spread = |px: [u8; 4]| {
            let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
            (r.max(g).max(b) - r.min(g).min(b)) as u32
        };
        assert!(spread(high) < 90, "低饱和主题着色后三通道应接近：{high:?}");
    }

    /// 压暗（l_mul<1）：同一主题色，挂起态应比守护态更暗
    #[test]
    fn darken_lowers_lightness() {
        let mut active = [191u8, 64, 42, 255];
        let mut suspended = [191u8, 64, 42, 255];
        tint(&mut active, 9.0, 0.8, 1.0);
        tint(&mut suspended, 9.0, 0.8, 0.6);
        let lum = |px: [u8; 4]| px[0] as u32 + px[1] as u32 + px[2] as u32;
        assert!(
            lum(suspended) < lum(active),
            "挂起({suspended:?}) 应暗于 守护({active:?})"
        );
    }

    /// 无彩色像素（黑白灰描边）：保持中性，只随 l_mul 压暗，不染色
    #[test]
    fn achromatic_stays_neutral() {
        let mut rgba = [128u8, 128, 128, 255];
        tint(&mut rgba, 200.0, 0.8, 0.6);
        assert_eq!(rgba[0], rgba[1], "仍为灰");
        assert_eq!(rgba[1], rgba[2], "仍为灰");
        assert!(rgba[0] < 128, "应压暗：{rgba:?}");
    }
}
