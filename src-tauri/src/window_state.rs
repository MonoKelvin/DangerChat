//! 主窗口位置/大小记忆（`config.json` 的 `window.*` 键，非 schema 管理）。
//!
//! 关窗 = 隐藏到托盘（前端 win.hide()），没有 Exit 事件可挂，
//! 官方 window-state 插件的那套「退出时保存」在这里不适用——
//! 改为在 Moved/Resized（节流）与失焦时主动落盘，启动时 show 之前恢复。
//!
//! 存进 `config.json`（[`ConfigCenter::set_unmanaged_batch`]，不进 schema）：
//! 与用户配置同文件、同一套原子写与损坏自愈，且不污染设置页表单。
//! 不记忆最大化状态：主窗口 `maximizable: false`（tauri.conf），不存在该态。
//!
//! # 两个必须守住的边界（否则窗口会「消失」）
//!
//! 1. **最小化时不得落盘**。Windows 把最小化窗口的矩形挪到 `-32000, -32000`
//!    这个占位坐标；若存下来，下次启动就会把窗口「恢复」到屏幕外——
//!    表现为任务栏有图标、窗口却永远不出现。
//! 2. **恢复位置前必须校验可达性**。显示器拔掉/改分辨率/多屏变化后，
//!    旧坐标可能落在任何显示器之外，同样导致窗口不可见。
//!    校验不过就只恢复尺寸，位置回落到 tauri.conf 的居中。

use dc_bridge::state::AppState;
use dc_core::{ConfigCenter, ConfigValue};

const KEY_X: &str = "window.x";
const KEY_Y: &str = "window.y";
const KEY_WIDTH: &str = "window.width";
const KEY_HEIGHT: &str = "window.height";

/// 最小尺寸，与 tauri.conf 的 minWidth/minHeight 保持一致。
const MIN_W: i32 = 760;
const MIN_H: i32 = 520;

/// 自定义标题栏（唯一拖拽区）的高度，与前端 `header` 的 `h-11` 一致。
const HEADER_H: i32 = 44;
/// 头部条至少要在显示器内露出这么多，才认为「用户够得着、能拖回来」。
const MIN_VISIBLE_W: i32 = 120;
const MIN_VISIBLE_H: i32 = 16;

/// 显示器矩形（物理像素）：`(x, y, w, h)`。
type Rect = (i32, i32, u32, u32);

/// 窗口的头部条是否与任一显示器有足够交集。
///
/// 只校验**头部条**而非整窗面积：头部是唯一拖拽区，它可达就一定能拖回来，
/// 比按整窗面积判断更准确（整窗只露出一角时其实用户无法拖动）。
///
/// 纯函数（显示器矩形由调用方注入），便于对「最小化占位坐标」「显示器变更」
/// 等无法在单测里造真窗口的场景做断言。
fn header_reachable(pos: (i32, i32), size: (u32, u32), monitors: &[Rect]) -> bool {
    let (px, py) = pos;
    let (pw, ph) = (size.0 as i32, size.1 as i32);
    // 头部条高度不会超过窗口总高（极小窗口时按总高算）
    let header_h = ph.min(HEADER_H);

    monitors.iter().any(|(mx, my, mw, mh)| {
        let (mx1, my1) = (mx + (*mw as i32), my + (*mh as i32));
        // 头部条与显示器矩形的交集尺寸
        let ix = (px + pw).min(mx1) - px.max(*mx);
        let iy = (py + header_h).min(my1) - py.max(*my);
        ix >= MIN_VISIBLE_W && iy >= MIN_VISIBLE_H
    })
}

/// 旧版独立 window-state.json（已废弃）一次性迁入 config 后删除。
///
/// 仅在 config 里还没有 window 键时导入（json 若比 config 旧，不得反过来覆盖）。
fn migrate_legacy_json(config: &ConfigCenter, data_dir: &std::path::Path) {
    #[derive(serde::Deserialize)]
    struct Legacy {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        #[serde(default)]
        #[allow(dead_code)] // 旧格式带 maximized，新版无此态，解析后丢弃
        maximized: bool,
    }

    let path = data_dir.join("window-state.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let snapshot = config.snapshot();
    if [KEY_X, KEY_Y, KEY_WIDTH, KEY_HEIGHT]
        .iter()
        .all(|k| snapshot.contains(k))
    {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let Ok(legacy) = serde_json::from_str::<Legacy>(&text) else {
        let _ = std::fs::remove_file(&path); // 解析不动也没必要留着
        return;
    };
    let _ = config.set_unmanaged_batch(vec![
        (KEY_X.into(), ConfigValue::Int(legacy.x as i64)),
        (KEY_Y.into(), ConfigValue::Int(legacy.y as i64)),
        (KEY_WIDTH.into(), ConfigValue::Int(legacy.width as i64)),
        (KEY_HEIGHT.into(), ConfigValue::Int(legacy.height as i64)),
    ]);
    let _ = std::fs::remove_file(&path);
}

/// 读取并保存当前窗口状态；失败静默（记忆是锦上添花，不拖累主流程）。
pub fn snapshot(win: &tauri::WebviewWindow, config: &ConfigCenter) {
    // 最小化时 Windows 报的是 -32000,-32000 占位矩形，存下来会把下次启动的窗口
    // 丢到屏幕外。此时**保持上一次的有效状态**，不覆盖。
    if win.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(size), Ok(pos)) = (win.outer_size(), win.outer_position()) else {
        return;
    };
    if let Err(e) = config.set_unmanaged_batch(vec![
        (KEY_X.into(), ConfigValue::Int(pos.x as i64)),
        (KEY_Y.into(), ConfigValue::Int(pos.y as i64)),
        (KEY_WIDTH.into(), ConfigValue::Int(size.width as i64)),
        (KEY_HEIGHT.into(), ConfigValue::Int(size.height as i64)),
    ]) {
        tracing::warn!(error = ?e, "窗口状态保存失败（不影响运行）");
    }
}

/// 启动恢复（show 之前调用，用户无感）。最小尺寸钳制与 tauri.conf 一致。
pub fn restore(win: &tauri::WebviewWindow, state: &AppState) {
    migrate_legacy_json(&state.config, &state.data_dir);
    let snap = state.config.snapshot();
    let (Some(x), Some(y), Some(width), Some(height)) = (
        snap.get(KEY_X).and_then(|v| v.as_i64()),
        snap.get(KEY_Y).and_then(|v| v.as_i64()),
        snap.get(KEY_WIDTH).and_then(|v| v.as_i64()),
        snap.get(KEY_HEIGHT).and_then(|v| v.as_i64()),
    ) else {
        return;
    };
    let size = tauri::PhysicalSize::new(
        (width as i32).max(MIN_W) as u32,
        (height as i32).max(MIN_H) as u32,
    );
    let _ = win.set_size(size);

    // 位置可达性校验：显示器拔掉/换分辨率/多屏变化后旧坐标可能已在屏幕外，
    // 直接 set_position 会让窗口「有图标却看不见」。取不到显示器列表时同样
    // 走保守分支（宁可居中，也不赌一个可能无效的坐标）。
    let monitors: Vec<Rect> = win
        .available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    let p = m.position();
                    let s = m.size();
                    (p.x, p.y, s.width, s.height)
                })
                .collect()
        })
        .unwrap_or_default();

    let pos = (x as i32, y as i32);
    if header_reachable(pos, (size.width, size.height), &monitors) {
        let _ = win.set_position(tauri::PhysicalPosition::new(pos.0, pos.1));
    } else {
        // 只恢复尺寸，位置交给 tauri.conf 的 center: true
        tracing::warn!(
            x = pos.0,
            y = pos.1,
            monitor_count = monitors.len(),
            "记忆的窗口位置不可达（屏幕外或显示器已变更），回落到居中"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单显示器 1920x1080（主屏原点）。
    const SINGLE: [Rect; 1] = [(0, 0, 1920, 1080)];

    /// 最小化时 Windows 报的占位坐标必须被判定为不可达 —— 这是「窗口消失」的根因。
    #[test]
    fn minimized_sentinel_position_is_rejected() {
        assert!(!header_reachable((-32000, -32000), (920, 610), &SINGLE));
    }

    /// 正常落屏的位置必须通过（否则记忆功能等于被关掉）。
    #[test]
    fn on_screen_position_is_accepted() {
        assert!(header_reachable((100, 100), (920, 610), &SINGLE));
        // 贴右下角、部分在屏幕外但头部可见 → 仍可用
        assert!(header_reachable((1500, 800), (920, 610), &SINGLE));
        // 负坐标但头部仍在屏内（窗口上沿略微出屏）→ 可拖回来
        assert!(header_reachable((0, -8), (920, 610), &SINGLE));
    }

    /// 完全在屏幕外（显示器被拔掉后的典型残留）必须拒绝。
    #[test]
    fn off_screen_position_is_rejected() {
        // 原第二显示器在 x=1920..3840，拔掉后这些坐标已无显示器
        assert!(!header_reachable((2400, 300), (920, 610), &SINGLE));
        // 整窗在屏幕下方之外
        assert!(!header_reachable((100, 1200), (920, 610), &SINGLE));
        // 整窗在屏幕左侧之外
        assert!(!header_reachable((-2000, 100), (920, 610), &SINGLE));
    }

    /// 头部条只露出不到阈值（用户够不着拖拽区）必须拒绝。
    #[test]
    fn barely_visible_header_is_rejected() {
        // 竖直方向只露出 4px（< MIN_VISIBLE_H=16）
        assert!(!header_reachable((100, -606), (920, 610), &SINGLE));
        // 水平方向只露出 20px（< MIN_VISIBLE_W=120）
        assert!(!header_reachable((-900, 100), (920, 610), &SINGLE));
    }

    /// 多显示器：落在任一显示器上都算可达。
    #[test]
    fn second_monitor_is_accepted() {
        let dual: [Rect; 2] = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        assert!(header_reachable((2400, 300), (920, 610), &dual));
        // 上置显示器（负 y）同样算数
        let stacked: [Rect; 2] = [(0, 0, 1920, 1080), (0, -1080, 1920, 1080)];
        assert!(header_reachable((100, -900), (920, 610), &stacked));
    }

    /// 取不到显示器列表（API 失败）时不得误判为可达 —— 保守回落到居中。
    #[test]
    fn empty_monitor_list_is_rejected() {
        assert!(!header_reachable((100, 100), (920, 610), &[]));
    }
}
