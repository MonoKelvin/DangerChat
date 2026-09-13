//! M0 最小风险提示窗口。
//!
//! 用途（`docs/M0_技术尖峰状态.md` 下一步 §3）：
//! 测量「物理按键 → 提示可见」的完整 M0-D 延迟，以及 `docs/01` §15 要求的
//! 「100 ms 内出现处理中反馈」。没有这个窗口，延迟测量就缺少最后一段，
//! 无法判定 M0-D 通过。
//!
//! 这是尖峰阶段的测量设施，不是产品 UI：
//! - 只显示固定的状态文字，不渲染任何草稿或会话内容。
//! - 不接受键盘输入、不提供按钮，因此不可能代替用户做出发送决定。
//! - 时间戳在 `WM_PAINT` 返回后采集，代表像素已提交给桌面合成器。

use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
    DEFAULT_PITCH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_NORMAL,
    HBRUSH, HDC, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    PostThreadMessageW, RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNA,
    WM_APP, WM_DESTROY, WM_PAINT, WM_QUIT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::platform::clock::MonotonicClock;

/// 提示窗口当前显示的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeState {
    /// 隐藏。
    Hidden = 0,
    /// 「正在检查」——按键被抑制后必须在 100 ms 内可见。
    Checking = 1,
    /// 「已检查，再按一次发送」。
    Allow = 2,
    /// 「发现风险」。
    Confirm = 3,
    /// 「检测不可用」。
    Unavailable = 4,
}

impl NoticeState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => NoticeState::Checking,
            2 => NoticeState::Allow,
            3 => NoticeState::Confirm,
            4 => NoticeState::Unavailable,
            _ => NoticeState::Hidden,
        }
    }

    fn text(self) -> &'static str {
        match self {
            NoticeState::Hidden => "",
            NoticeState::Checking => "正在检查…",
            NoticeState::Allow => "已检查，再按一次发送",
            NoticeState::Confirm => "发现风险，请确认",
            NoticeState::Unavailable => "检测不可用",
        }
    }

    fn background(self) -> COLORREF {
        match self {
            NoticeState::Confirm => COLORREF(0x00202080),
            NoticeState::Unavailable => COLORREF(0x00404040),
            _ => COLORREF(0x00302020),
        }
    }
}

/// 当前状态，由 UI 线程在绘制时读取。
static STATE: AtomicU8 = AtomicU8::new(0);
/// 最近一次绘制完成的 QPC 纳秒时间戳。`-1` 表示尚未绘制。
static PAINTED_AT: AtomicI64 = AtomicI64::new(-1);
/// 绘制完成时对应的状态，避免把上一状态的绘制误认成本次。
static PAINTED_STATE: AtomicU8 = AtomicU8::new(0);

const WM_NOTICE_UPDATE: u32 = WM_APP + 2;

/// 提示窗口句柄。UI 线程独占，其他线程通过消息驱动。
pub struct NoticeWindow {
    thread_id: u32,
    ui_thread: Option<JoinHandle<()>>,
    ready: Receiver<()>,
}

impl NoticeWindow {
    /// 在专用线程上创建提示窗口并等待其就绪。
    pub fn spawn() -> Result<Self, String> {
        let (tid_tx, tid_rx) = mpsc::channel::<Result<u32, String>>();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();

        let ui_thread = std::thread::Builder::new()
            .name("m0-notice".into())
            .spawn(move || run_ui_thread(tid_tx, ready_tx))
            .map_err(|e| format!("无法创建提示窗口线程: {e}"))?;

        let thread_id = tid_rx
            .recv()
            .map_err(|e| format!("提示窗口线程未响应: {e}"))??;

        Ok(Self {
            thread_id,
            ui_thread: Some(ui_thread),
            ready: ready_rx,
        })
    }

    /// 请求切换状态。立即返回，不等待绘制。
    ///
    /// 调用方随后用 `wait_until_visible` 取得像素可见的时间戳。
    pub fn request(&self, state: NoticeState) {
        STATE.store(state as u8, Ordering::Release);
        // SAFETY: 目标是本进程自己的 UI 线程，不是任何外部窗口。
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_NOTICE_UPDATE, WPARAM(0), LPARAM(0));
        }
    }

    /// 等待指定状态完成绘制，返回绘制完成的 QPC 纳秒时间戳。
    ///
    /// 超时返回 `None`，调用方必须按失败处理，不得用请求时刻代替可见时刻。
    pub fn wait_until_visible(
        &self,
        state: NoticeState,
        clock: &MonotonicClock,
        timeout_nanos: u64,
    ) -> Option<u64> {
        let deadline = clock.now_nanos() + timeout_nanos;
        loop {
            let painted = PAINTED_AT.load(Ordering::Acquire);
            if painted >= 0 && PAINTED_STATE.load(Ordering::Acquire) == state as u8 {
                return Some(painted as u64);
            }
            if clock.now_nanos() >= deadline {
                return None;
            }
            std::thread::yield_now();
        }
    }
}

impl Drop for NoticeWindow {
    fn drop(&mut self) {
        // SAFETY: 目标是本进程自己的 UI 线程。
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.ui_thread.take() {
            let _ = thread.join();
        }
        let _ = self.ready.try_recv();
    }
}

fn run_ui_thread(tid_tx: Sender<Result<u32, String>>, ready_tx: Sender<()>) {
    // SAFETY: 标准窗口注册与创建流程，全部作用于本进程自己的窗口。
    let hwnd = unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            let _ = tid_tx.send(Err("无法取得模块句柄".into()));
            return;
        };
        let class_name = w!("DangerChatM0Notice");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(notice_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        RegisterClassW(&class);

        // WS_EX_NOACTIVATE：提示窗永不抢焦点，用户按键始终发往原目标窗口。
        let created = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("DangerChat 提示"),
            WS_POPUP,
            40,
            40,
            360,
            96,
            None,
            None,
            Some(instance.into()),
            None,
        );
        match created {
            Ok(hwnd) => hwnd,
            Err(e) => {
                let _ = tid_tx.send(Err(format!("无法创建提示窗口: {e}")));
                return;
            }
        }
    };

    // SAFETY: GetCurrentThreadId 无参数、无副作用。
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    if tid_tx.send(Ok(tid)).is_err() {
        // SAFETY: 销毁本线程刚创建的窗口。
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return;
    }
    let _ = ready_tx.send(());

    // SAFETY: 置顶但不激活，保证不夺取目标窗口的前台状态。
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }

    let mut msg = MSG::default();
    loop {
        // SAFETY: 标准消息循环，msg 为本地栈变量。
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        if msg.message == WM_NOTICE_UPDATE {
            apply_state(hwnd);
            continue;
        }
        // SAFETY: 转发本线程自身的消息。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // SAFETY: 销毁本线程创建的窗口。
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn apply_state(hwnd: HWND) {
    let state = NoticeState::from_u8(STATE.load(Ordering::Acquire));
    // 进入新状态前清除旧的绘制时间戳，避免把上一次绘制当作本次可见。
    PAINTED_AT.store(-1, Ordering::Release);

    // SAFETY: 只操作本线程创建的窗口。
    unsafe {
        if state == NoticeState::Hidden {
            let _ = ShowWindow(hwnd, SW_HIDE);
            PAINTED_STATE.store(NoticeState::Hidden as u8, Ordering::Release);
            PAINTED_AT.store(0, Ordering::Release);
            return;
        }
        // SW_SHOWNA：显示但不激活。
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
    }
}

unsafe extern "system" fn notice_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let state = NoticeState::from_u8(STATE.load(Ordering::Acquire));
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: 标准 WM_PAINT 处理，BeginPaint/EndPaint 成对调用。
            unsafe {
                let hdc = BeginPaint(hwnd, &mut ps);
                paint_notice(hdc, ps.rcPaint, state);
                let _ = EndPaint(hwnd, &ps);
            }
            // EndPaint 之后像素已提交，此刻即「提示可见」。
            let clock = MonotonicClock::new();
            PAINTED_STATE.store(state as u8, Ordering::Release);
            PAINTED_AT.store(clock.now_nanos() as i64, Ordering::Release);
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 退出本窗口的消息循环。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: 其余消息交回默认处理。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint_notice(hdc: HDC, rect: RECT, state: NoticeState) {
    // SAFETY: 所有 GDI 对象在使用后立即释放；绘制目标是本窗口的 DC。
    unsafe {
        let brush = CreateSolidBrush(state.background());
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush.into());

        SetBkMode(hdc, windows::Win32::Graphics::Gdi::TRANSPARENT);
        SetTextColor(hdc, COLORREF(0x00F0F0F0));

        let face: Vec<u16> = "Microsoft YaHei UI"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let font = CreateFontW(
            24,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            PCWSTR(face.as_ptr()),
        );
        let old = SelectObject(hdc, font.into());
        let mut buffer: Vec<u16> = state.text().encode_utf16().collect();
        let mut r = rect;
        DrawTextW(
            hdc,
            &mut buffer,
            &mut r,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        SelectObject(hdc, old);
        let _ = DeleteObject(font.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_u8() {
        for state in [
            NoticeState::Hidden,
            NoticeState::Checking,
            NoticeState::Allow,
            NoticeState::Confirm,
            NoticeState::Unavailable,
        ] {
            assert_eq!(NoticeState::from_u8(state as u8), state);
        }
    }

    #[test]
    fn unknown_discriminant_falls_back_to_hidden() {
        assert_eq!(NoticeState::from_u8(200), NoticeState::Hidden);
    }

    #[test]
    fn only_hidden_renders_empty_text() {
        assert!(NoticeState::Hidden.text().is_empty());
        for state in [
            NoticeState::Checking,
            NoticeState::Allow,
            NoticeState::Confirm,
            NoticeState::Unavailable,
        ] {
            assert!(!state.text().is_empty());
        }
    }
}
