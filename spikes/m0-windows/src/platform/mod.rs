//! Windows 平台适配层。唯一允许出现 `unsafe` 的位置。
//!
//! 边界约束（`docs/合规与风险说明.md` §4）：
//! - 只安装本进程的 `WH_KEYBOARD_LL`，不向任何进程注入 DLL。
//! - 只读取操作系统已持有的窗口元数据，不枚举、不扫描、不读取目标进程内容。
//! - 捕获只使用显示器级接口，不使用窗口定向捕获、UIA 或 `PrintWindow`。
//! - 不存在输入注入接口：本层不导出、也不调用 `SendInput`/`keybd_event`/
//!   `mouse_event`，以及任何向外部窗口投递消息的函数。

pub mod capture;
pub mod clock;
pub mod hook;
pub mod ime;
pub mod layout;
pub mod window;
