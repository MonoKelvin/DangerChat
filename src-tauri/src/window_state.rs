//! 主窗口位置/大小记忆（window-state.json，数据目录根）。
//!
//! 关窗 = 隐藏到托盘（前端 win.hide()），没有 Exit 事件可挂，
//! 官方 window-state 插件的那套「退出时保存」在这里不适用——
//! 改为在 Moved/Resized（节流）与失焦时主动落盘，启动时 show 之前恢复。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowState {
    /// 物理像素（跨 DPI 变化不做逻辑换算：还原优先保证像素级一致）
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

fn path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("window-state.json")
}

pub fn load(data_dir: &std::path::Path) -> Option<WindowState> {
    let text = std::fs::read_to_string(path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(data_dir: &std::path::Path, state: &WindowState) {
    if let Ok(text) = serde_json::to_string(state) {
        if let Err(e) = std::fs::write(path(data_dir), text) {
            tracing::warn!(error = %e, "窗口状态保存失败（不影响运行）");
        }
    }
}

/// 读取并保存当前窗口状态；失败静默（记忆是锦上添花，不拖累主流程）。
pub fn snapshot(win: &tauri::WebviewWindow, data_dir: &std::path::Path) {
    let (Ok(size), Ok(pos)) = (win.outer_size(), win.outer_position()) else {
        return;
    };
    save(
        data_dir,
        &WindowState {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: win.is_maximized().unwrap_or(false),
        },
    );
}

/// 启动恢复（show 之前调用，用户无感）。最小尺寸钳制与 tauri.conf 一致。
pub fn restore(win: &tauri::WebviewWindow, data_dir: &std::path::Path) {
    let Some(ws) = load(data_dir) else {
        return;
    };
    let width = ws.width.max(760);
    let height = ws.height.max(520);
    let _ = win.set_size(tauri::PhysicalSize::new(width, height));
    let _ = win.set_position(tauri::PhysicalPosition::new(ws.x, ws.y));
    if ws.maximized {
        let _ = win.maximize();
    }
}
