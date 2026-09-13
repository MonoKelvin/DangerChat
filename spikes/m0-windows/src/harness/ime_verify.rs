//! M0-B 输入法状态判定的自动验证。
//!
//! 目的（`docs/02` §7.3）：输入法处于组合或候选状态时，Enter 的语义是
//! 「确认候选」而非「发送消息」，此时必须透传，绝不能误吞。
//!
//! ## 为什么可以自动验证
//!
//! 判定的输入只有两个：IMM32 查询结果与守卫的 `ime_composing` 标志。
//! 前者可以在自有窗口上程序化构造真实的 IMM32 上下文状态
//! （打开/关闭输入法、写入组合串），后者是纯逻辑。
//! 两者都不需要人工敲键盘。
//!
//! ## 仍需人工的部分
//!
//! 本验证覆盖 IMM32 路径。若某输入法走 TSF 而不更新 IMM32 组合串，
//! 本验证无法发现——那种情况下 `probe_window` 会返回 `Unknown`，
//! 而 `Unknown` 已映射为「透传」，属于安全侧失败。
//! 真实输入法的候选行为差异仍建议用 `ime-hook-probe` 人工抽查。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    ImmAssociateContext, ImmCreateContext, ImmDestroyContext, ImmGetCompositionStringW,
    ImmGetContext, ImmReleaseContext, ImmSetCompositionStringW, ImmSetOpenStatus, GCS_COMPSTR,
    HIMC, SCS_SETSTR,
};

use crate::platform::ime::{probe_window, ImeState};

/// 单个场景的验证结果。
#[derive(Debug, Clone)]
pub struct ImeCase {
    pub label: &'static str,
    pub observed: ImeState,
    /// 该状态下守卫是否会把发送键让给输入法。
    pub defers: bool,
    /// 是否符合预期。
    pub passed: bool,
    /// 构造该场景时的补充说明（例如 IMM32 拒绝写入组合串）。
    pub note: Option<String>,
}

/// 在自有窗口上构造 IMM32 状态并验证判定。
///
/// 窗口由调用方提供，必须是本进程自己创建的窗口。
pub fn verify(hwnd: HWND) -> Result<Vec<ImeCase>, String> {
    let mut cases = Vec::new();

    // 场景 1：输入法上下文关闭。这是纯英文直接输入的典型状态。
    let (closed, _) = with_ime_context(hwnd, false, None, || probe_window(hwnd))?;
    cases.push(judge(
        "输入法关闭（英文直接输入）",
        closed,
        None,
        // 关闭时应能明确判定「未组合」，允许拦截发送键。
        |s| s == ImeState::NotComposing,
    ));

    // 场景 2：输入法打开但无组合串。候选可能仍在，必须保守。
    let (open_empty, _) = with_ime_context(hwnd, true, None, || probe_window(hwnd))?;
    cases.push(judge(
        "输入法打开、无组合串",
        open_empty,
        None,
        // 不能断言「没有组合」，必须让给输入法。
        |s| s.should_defer_to_ime(),
    ));

    // 场景 3：尝试写入组合串。
    //
    // `ImmSetCompositionStringW` 只对拥有输入法焦点的上下文生效；
    // 即使它返回 TRUE，在无 IME 焦点的测试窗口上组合串也可能读不回来。
    // 因此这里回读校验，区分「真的构造出组合态」与「场景未成立」，
    // 不允许后者冒充已验证。
    let (composing, wrote) = with_ime_context(hwnd, true, Some("nihao"), || {
        (probe_window(hwnd), composition_len(hwnd))
    })?;
    let (observed, readback) = composing;
    let note = match (wrote, readback) {
        (_, len) if len > 0 => None,
        (true, _) => {
            Some("IMM32 接受写入但组合串读不回（无 IME 焦点），本轮未真正构造组合态".to_string())
        }
        (false, _) => Some("IMM32 拒绝写入组合串，本轮未真正构造组合态".to_string()),
    };
    cases.push(judge(
        "写入组合串（拼音未确认）",
        observed,
        note,
        // 无论构造是否成立，都不得判为「可拦截」。
        |s| s.should_defer_to_ime(),
    ));

    // 场景 4：无输入法上下文。无法证明没有组合，按不确定处理。
    let detached = without_ime_context(hwnd, || probe_window(hwnd));
    cases.push(judge("窗口无输入法上下文", detached, None, |s| {
        s.should_defer_to_ime()
    }));

    Ok(cases)
}

fn judge(
    label: &'static str,
    observed: ImeState,
    note: Option<String>,
    expected: impl Fn(ImeState) -> bool,
) -> ImeCase {
    ImeCase {
        label,
        observed,
        defers: observed.should_defer_to_ime(),
        passed: expected(observed),
        note,
    }
}

/// 在给定 IMM32 状态下执行闭包，结束后恢复原状态。
///
/// 返回闭包结果与「组合串是否真的写入成功」。后者用于区分
/// 「验证了组合态」与「IMM32 拒绝写入、场景未成立」。
fn with_ime_context<T>(
    hwnd: HWND,
    open: bool,
    composition: Option<&str>,
    body: impl FnOnce() -> T,
) -> Result<(T, bool), String> {
    // SAFETY: 全部作用于本进程自己的窗口与其输入法上下文。
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.is_invalid() {
            return Err("窗口没有输入法上下文；请先创建带 IME 的窗口".into());
        }

        let _ = ImmSetOpenStatus(context, open);

        let mut wrote = false;
        if let Some(text) = composition {
            let wide: Vec<u16> = text.encode_utf16().collect();
            let bytes = std::mem::size_of_val(wide.as_slice()) as u32;
            wrote = ImmSetCompositionStringW(
                context,
                SCS_SETSTR,
                Some(wide.as_ptr().cast()),
                bytes,
                None,
                0,
            )
            .as_bool();
        }

        let result = body();

        // 清理：撤销组合串并关闭输入法，避免影响后续场景。
        let _ = ImmSetCompositionStringW(context, SCS_SETSTR, None, 0, None, 0);
        let _ = ImmSetOpenStatus(context, false);
        let _ = ImmReleaseContext(hwnd, context);
        Ok((result, wrote))
    }
}

/// 读取当前组合串的字节长度。用于回读校验场景是否真的成立。
fn composition_len(hwnd: HWND) -> i32 {
    // SAFETY: 成对获取/释放上下文；传 None 时只返回所需字节数。
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.is_invalid() {
            return -1;
        }
        let len = ImmGetCompositionStringW(context, GCS_COMPSTR, None, 0);
        let _ = ImmReleaseContext(hwnd, context);
        len
    }
}

/// 临时解除窗口的输入法上下文关联，执行闭包后恢复。
fn without_ime_context<T>(hwnd: HWND, body: impl FnOnce() -> T) -> T {
    // SAFETY: 只操作本进程自己窗口的 IMM32 关联，结束后恢复。
    unsafe {
        let previous = ImmAssociateContext(hwnd, HIMC::default());
        let result = body();
        ImmAssociateContext(hwnd, previous);
        result
    }
}

/// 创建一个带输入法上下文的隐藏窗口，供验证使用。
pub fn create_ime_context(hwnd: HWND) -> Result<HIMC, String> {
    // SAFETY: 为本进程自己的窗口创建并关联输入法上下文。
    unsafe {
        let context = ImmCreateContext();
        if context.is_invalid() {
            return Err("无法创建输入法上下文".into());
        }
        ImmAssociateContext(hwnd, context);
        Ok(context)
    }
}

/// 销毁由 `create_ime_context` 创建的上下文。
pub fn destroy_ime_context(hwnd: HWND, context: HIMC) {
    // SAFETY: 与 create_ime_context 配对。
    unsafe {
        ImmAssociateContext(hwnd, HIMC::default());
        let _ = ImmDestroyContext(context);
    }
}
