//! 物理按键身份与发送快捷键匹配。
//!
//! 纯逻辑，不依赖 Windows，可在任意平台单测。

/// 物理按键身份：`vkCode + scanCode + extended`。
///
/// 仅用 `vkCode` 不足以区分主键盘 Enter 与小键盘 Enter，
/// 因此配对 keydown/keyup 必须使用完整三元组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalKey {
    pub vk: u16,
    pub scan: u16,
    pub extended: bool,
}

impl PhysicalKey {
    pub const fn new(vk: u16, scan: u16, extended: bool) -> Self {
        Self { vk, scan, extended }
    }
}

pub const VK_RETURN: u16 = 0x0D;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_MENU: u16 = 0x12;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;

/// 当前物理修饰键状态。由守卫自行跟踪，不查询异步键盘状态。
///
/// `LowLevelKeyboardProc` 在按键异步状态更新之前被调用，
/// 所以回调内不能用 `GetAsyncKeyState` 判断修饰键。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

impl Modifiers {
    /// 除 ctrl 外是否按下了其他修饰键。
    pub fn has_extra_beyond_ctrl(self) -> bool {
        self.shift || self.alt || self.win
    }

    pub fn any(self) -> bool {
        self.ctrl || self.shift || self.alt || self.win
    }

    pub fn apply(&mut self, vk: u16, down: bool) {
        match vk {
            VK_CONTROL => self.ctrl = down,
            VK_SHIFT => self.shift = down,
            VK_MENU => self.alt = down,
            VK_LWIN | VK_RWIN => self.win = down,
            _ => {}
        }
    }
}

/// 用户配置的发送快捷键。三者互斥，必须精确匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// 主键盘 Enter，无任何修饰键。
    Enter,
    /// 小键盘 Enter（extended），无任何修饰键。
    NumpadEnter,
    /// Ctrl + Enter（主键盘或小键盘），无其他修饰键。
    CtrlEnter,
}

impl Shortcut {
    /// 判断一次按键是否精确匹配本快捷键。
    ///
    /// 存在未配置的额外修饰键时一律不匹配 —— 调用方据此透传，
    /// 避免吞掉用户在其他程序里的组合键。
    pub fn matches(self, key: PhysicalKey, mods: Modifiers) -> bool {
        if key.vk != VK_RETURN {
            return false;
        }
        match self {
            Shortcut::Enter => !key.extended && !mods.any(),
            Shortcut::NumpadEnter => key.extended && !mods.any(),
            Shortcut::CtrlEnter => mods.ctrl && !mods.has_extra_beyond_ctrl(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN_ENTER: PhysicalKey = PhysicalKey::new(VK_RETURN, 0x1C, false);
    const NUMPAD_ENTER: PhysicalKey = PhysicalKey::new(VK_RETURN, 0x1C, true);

    fn mods(ctrl: bool, shift: bool, alt: bool, win: bool) -> Modifiers {
        Modifiers {
            ctrl,
            shift,
            alt,
            win,
        }
    }

    #[test]
    fn enter_requires_main_keyboard_without_modifiers() {
        assert!(Shortcut::Enter.matches(MAIN_ENTER, Modifiers::default()));
        assert!(!Shortcut::Enter.matches(NUMPAD_ENTER, Modifiers::default()));
        assert!(!Shortcut::Enter.matches(MAIN_ENTER, mods(true, false, false, false)));
        assert!(!Shortcut::Enter.matches(MAIN_ENTER, mods(false, true, false, false)));
    }

    #[test]
    fn numpad_enter_requires_extended_flag() {
        assert!(Shortcut::NumpadEnter.matches(NUMPAD_ENTER, Modifiers::default()));
        assert!(!Shortcut::NumpadEnter.matches(MAIN_ENTER, Modifiers::default()));
    }

    #[test]
    fn ctrl_enter_rejects_extra_modifiers() {
        assert!(Shortcut::CtrlEnter.matches(MAIN_ENTER, mods(true, false, false, false)));
        assert!(Shortcut::CtrlEnter.matches(NUMPAD_ENTER, mods(true, false, false, false)));
        assert!(!Shortcut::CtrlEnter.matches(MAIN_ENTER, mods(true, true, false, false)));
        assert!(!Shortcut::CtrlEnter.matches(MAIN_ENTER, mods(true, false, true, false)));
        assert!(!Shortcut::CtrlEnter.matches(MAIN_ENTER, mods(true, false, false, true)));
        assert!(!Shortcut::CtrlEnter.matches(MAIN_ENTER, Modifiers::default()));
    }

    #[test]
    fn non_enter_keys_never_match() {
        let a = PhysicalKey::new(0x41, 0x1E, false);
        for sc in [Shortcut::Enter, Shortcut::NumpadEnter, Shortcut::CtrlEnter] {
            assert!(!sc.matches(a, Modifiers::default()));
            assert!(!sc.matches(a, mods(true, false, false, false)));
        }
    }

    #[test]
    fn modifier_tracking_follows_down_and_up() {
        let mut m = Modifiers::default();
        m.apply(VK_CONTROL, true);
        assert!(m.ctrl);
        m.apply(VK_CONTROL, false);
        assert!(!m.ctrl);
        m.apply(VK_LWIN, true);
        assert!(m.win);
        m.apply(VK_RWIN, false);
        assert!(!m.win);
    }
}
