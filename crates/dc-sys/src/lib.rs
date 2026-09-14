//! # dc-sys — 平台层（设计文档 §5.2）
//!
//! **本 crate 是业务代码中唯一允许直接调用 Win32 的地方**（Tauri 运行时及其官方插件——
//! 托盘/自启动/单实例——的内部 Win32 使用不在本模块管辖范围）。
//!
//! 对外只暴露 [`SysApi`] trait，提供 [`RealSys`]（真实桌面）与 [`MockSys`]（脚本化夹具）双实现，
//! 使 dc-pipeline 的全部逻辑可在无桌面、无微信、无输入设备的 CI 环境中测试。
//!
//! ## 合规约束（不可协商）
//!
//! - 本 crate **不存在**任何输入注入接口：键/鼠标注入类 API 全仓库零引用
//!   （清单见《合规与风险说明》§4.2；`tools/redline_scan` 在 CI 强制，零白名单例外）。
//! - 截图仅使用 `BitBlt` 对**屏幕 DC** 按窗口矩形拷贝屏幕像素；**禁止**任何窗口定向捕获
//!   （合规 C-05、§4.1；同样由红线扫描强制）。
//! - 前台检测只读公开 API（`GetForegroundWindow` / `SetWinEventHook`），不向目标程序发送任何事件。
//! - 前台/最小化状态的用途仅限于决定危信自身「挂起/唤醒」（C-06）。

mod api;
mod mock;
#[cfg(windows)]
mod real;

pub use api::{
    dpi_scale_from_dpi, to_physical, ForegroundCallback, ForegroundInfo, HookAction, HookGuard,
    Hwnd, KeyCallback, KeyEvent, Rect, SysApi, SysError, WatchGuard, VK_PROCESSKEY,
};
pub use mock::{MockSys, MockWindow};
#[cfg(windows)]
pub use real::{local_utc_offset_minutes, local_ymd, RealSys};

/// 构造函数：当前平台的真实实现。
#[cfg(windows)]
pub fn platform() -> RealSys {
    RealSys::new()
}
