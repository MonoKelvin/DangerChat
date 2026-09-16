//! 主窗口位置/大小记忆（config.toml 的 `window.*` 键，非 schema 管理）。
//!
//! 关窗 = 隐藏到托盘（前端 win.hide()），没有 Exit 事件可挂，
//! 官方 window-state 插件的那套「退出时保存」在这里不适用——
//! 改为在 Moved/Resized（节流）与失焦时主动落盘，启动时 show 之前恢复。
//!
//! 存进 config.toml（[`ConfigCenter::set_unmanaged_batch`]，不进 schema）：
//! 与用户配置同文件、同一套原子写与损坏自愈，且不污染设置页表单。
//! 不记忆最大化状态：主窗口 `maximizable: false`（tauri.conf），不存在该态。

use dc_bridge::state::AppState;
use dc_core::{ConfigCenter, ConfigValue};

const KEY_X: &str = "window.x";
const KEY_Y: &str = "window.y";
const KEY_WIDTH: &str = "window.width";
const KEY_HEIGHT: &str = "window.height";

/// 旧版独立 window-state.json（已废弃）一次性迁入 config.toml 后删除。
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
    let _ = win.set_size(tauri::PhysicalSize::new(
        width.max(760) as u32,
        height.max(520) as u32,
    ));
    let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
}
