//! 按键分类（§5.3 钩子回调的第一层判定）。
//!
//! 三件事必须区分清楚，否则会出现「吞掉用户正常输入」或「漏拦发送键」：
//! 1. 这是不是**发送键**（由 `guard.send_key` 配置决定 Enter / Ctrl+Enter）；
//! 2. 这是不是**内容修改键**（决定 `draft_epoch` 是否 +1，§2.2）；
//! 3. 这是不是**弹窗快捷键**（1/2/3/0，仅弹窗存续期由钩子代转，§5.3）。

use dc_sys::KeyEvent;

/// 虚拟键码（只列用到的）。
pub const VK_BACK: u16 = 0x08;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_PACKET: u16 = 0xE7;

/// 目标程序的发送快捷键（FR-SRC-06，默认 Enter）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendKey {
    #[default]
    Enter,
    CtrlEnter,
}

impl SendKey {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ctrl+enter" | "ctrl-enter" | "ctrlenter" => SendKey::CtrlEnter,
            _ => SendKey::Enter,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SendKey::Enter => "enter",
            SendKey::CtrlEnter => "ctrl+enter",
        }
    }

    /// 枚举取值（配置 schema 与前端下拉共用；parse 接受任意大小写）。
    pub fn options() -> Vec<String> {
        vec!["Enter".to_string(), "Ctrl+Enter".to_string()]
    }

    /// AtomicU8 编码（钩子路径热更新用）。
    pub fn encode(self) -> u8 {
        match self {
            SendKey::Enter => 0,
            SendKey::CtrlEnter => 1,
        }
    }

    pub fn decode(v: u8) -> Self {
        match v {
            1 => SendKey::CtrlEnter,
            _ => SendKey::Enter,
        }
    }
}

/// 是否为目标程序的发送键。
///
/// 区分修饰键是必要的：微信里「非发送快捷键」的回车是**换行**（输入法/微信自身处理），
/// 例如 `send_key = "enter"` 时 `Ctrl+Enter` 与 `Shift+Enter` 都必须放行。
pub fn is_send_key(ev: &KeyEvent, send_key: SendKey) -> bool {
    if ev.vk != VK_RETURN || ev.alt {
        return false;
    }
    match send_key {
        SendKey::Enter => !ev.ctrl && !ev.shift,
        SendKey::CtrlEnter => ev.ctrl && !ev.shift,
    }
}

/// 是否属于「会改变输入框内容」的按键（§2.2：可打印字符、Backspace、Delete、Ctrl+V、IME 提交）。
///
/// 用**白名单**而非区间判断：`0x20..=0x7E` 这种区间会把 F1~F12（0x70~0x7B）和
/// 方向键/Home/End（0x21~0x2D）一起误判成内容键，导致每按一次方向键就多算一纪元。
pub fn is_content_key(vk: u16) -> bool {
    match vk {
        VK_BACK | VK_DELETE | VK_PACKET => true, // 删除类 + 输入法 Unicode 提交
        0x20 => true,                            // 空格
        0x30..=0x39 => true,                     // 0-9
        0x41..=0x5A => true,                     // A-Z（Ctrl+V 也在此列）
        0x60..=0x6F => true,                     // 小键盘数字与运算键
        0xBA..=0xC0 => true,                     // OEM ; = , - . / `
        0xDB..=0xDE => true,                     // OEM [ \ ] '
        _ => false,
    }
}

/// 弹窗动作（FR-BRG-02 + §5.10：数字键 1/2/3 对应前三个按钮，0 对应「本次不再提示」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertAction {
    /// 仍然发送：置 allow-once 放行标志（**不代发**，红线 C-08）
    Allow,
    /// 取消发送
    Cancel,
    /// 返回编辑
    Edit,
    /// 本次不再提示（只静默当前草稿纪元，改稿即恢复）
    Snooze,
}

impl AlertAction {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertAction::Allow => "allow",
            AlertAction::Cancel => "cancel",
            AlertAction::Edit => "edit",
            AlertAction::Snooze => "snooze",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" => Some(AlertAction::Allow),
            "cancel" => Some(AlertAction::Cancel),
            "edit" => Some(AlertAction::Edit),
            "snooze" => Some(AlertAction::Snooze),
            _ => None,
        }
    }
}

/// 弹窗存续期的数字快捷键映射。要求不带修饰键（`Ctrl+1` 不是弹窗动作）。
pub fn alert_shortcut(ev: &KeyEvent) -> Option<AlertAction> {
    if ev.ctrl || ev.alt || ev.shift {
        return None;
    }
    match ev.vk {
        0x31 => Some(AlertAction::Allow),
        0x32 => Some(AlertAction::Cancel),
        0x33 => Some(AlertAction::Edit),
        0x30 => Some(AlertAction::Snooze),
        _ => None,
    }
}
