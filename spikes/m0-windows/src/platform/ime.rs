//! 输入法组合状态探测。
//!
//! 目的（`docs/02_危信v1.0_技术架构与接口设计.md` §7.3）：
//! 在输入法处于组合或候选状态时，Enter 的语义是“确认候选”而非“发送消息”。
//! 无法可靠区分时必须透传，绝不能误吞用户的候选确认。
//!
//! 边界：只查询输入法上下文与输入法窗口的状态，
//! 不读取目标程序的控件树、内容或任何内部数据。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmGetOpenStatus, ImmReleaseContext, GCS_COMPSTR,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// 输入法状态判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeState {
    /// 明确没有进行中的组合：可以按配置判定发送键。
    NotComposing,
    /// 明确处于组合/候选状态：Enter 属于输入法，必须透传。
    Composing,
    /// 无法确定。按“不确定即透传”处理，宁可漏保护也不误吞。
    Unknown,
}

impl ImeState {
    /// 是否应当把发送键让给输入法。
    ///
    /// `Unknown` 也返回 true：这是产品的安全默认值。
    pub fn should_defer_to_ime(self) -> bool {
        !matches!(self, ImeState::NotComposing)
    }
}

/// 探测当前前台窗口的输入法组合状态。
pub fn probe_foreground() -> ImeState {
    // SAFETY: GetForegroundWindow 无参数、只读。
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return ImeState::Unknown;
    }
    probe_window(hwnd)
}

/// 探测指定窗口的输入法组合状态。
pub fn probe_window(hwnd: HWND) -> ImeState {
    // SAFETY: ImmGetContext/ImmReleaseContext 成对调用；
    // ImmGetCompositionStringW 传 None 时只返回所需字节数，不写任何缓冲区。
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.is_invalid() {
            // 该窗口没有输入法上下文。常见于纯英文输入或不使用 IMM 的窗口。
            // 无法证明"没有组合"，按不确定处理。
            return ImeState::Unknown;
        }

        let open = ImmGetOpenStatus(context).as_bool();
        // 组合串长度 > 0 表示正在组合（拼音、五笔等候选未确认）。
        let comp_len = ImmGetCompositionStringW(context, GCS_COMPSTR, None, 0);

        let _ = ImmReleaseContext(hwnd, context);

        if comp_len > 0 {
            return ImeState::Composing;
        }
        if comp_len < 0 {
            // 负值为错误码，无法判定。
            return ImeState::Unknown;
        }
        if open {
            // 输入法已开启但当前没有组合串。
            // 候选窗口可能仍然打开（部分输入法的候选不体现在组合串上），
            // 因此不能断言"没有组合"，保持不确定。
            return ImeState::Unknown;
        }
        ImeState::NotComposing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_defers_to_ime() {
        assert!(ImeState::Unknown.should_defer_to_ime());
    }

    #[test]
    fn composing_defers_to_ime() {
        assert!(ImeState::Composing.should_defer_to_ime());
    }

    #[test]
    fn not_composing_allows_send_key_handling() {
        assert!(!ImeState::NotComposing.should_defer_to_ime());
    }
}
