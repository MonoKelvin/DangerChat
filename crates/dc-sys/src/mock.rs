//! `MockSys`：脚本化的 [`SysApi`] 实现，供全部单元测试使用（§11.1「Win32 一律 MockSys」）。
//!
//! 模型说明：
//! - 屏幕：一张由测试给定的 RGBA「虚拟桌面」，`capture_region` 从中按矩形裁剪；
//! - 窗口：以 **DIP 矩形 + DPI 缩放比** 登记，`window_rect` 返回换算后的物理矩形
//!   ——这正是在真机上「DIP 设计稿 → 物理像素」的那一步，UT-SYS-01 据此断言；
//! - 按键与前台事件：回调被存下来，由测试用 [`MockSys::feed_key`] / [`MockSys::emit_foreground`] 主动投递。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use image::RgbaImage;

use crate::api::{
    to_physical, ForegroundCallback, ForegroundInfo, HookAction, HookGuard, Hwnd, KeyCallback,
    KeyEvent, Rect, SysApi, SysError, WatchGuard,
};

/// 登记的虚拟窗口。
#[derive(Debug, Clone, PartialEq)]
pub struct MockWindow {
    /// 逻辑（DIP）矩形。
    pub rect_dip: Rect,
    pub dpi_scale: f32,
    pub minimized: bool,
}

impl MockWindow {
    pub fn new(rect_dip: Rect, dpi_scale: f32) -> Self {
        Self {
            rect_dip,
            dpi_scale,
            minimized: false,
        }
    }
}

#[derive(Default)]
struct State {
    screen: Option<RgbaImage>,
    windows: HashMap<Hwnd, MockWindow>,
    foreground: Option<ForegroundInfo>,
    /// 前台回调表：`(token, callback)`，全部调用（无「吞掉」语义）。
    foreground_cbs: Vec<(u64, ForegroundCallback)>,
    /// 键盘回调表：按注册顺序调用，任一 `Swallow` 即终止分发——与 RealSys 语义一致。
    key_cbs: Vec<(u64, KeyCallback)>,
    next_token: u64,
}

impl State {
    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }
}

struct Inner {
    state: Mutex<State>,
    ime_composing: AtomicBool,
    capture_fail_hard: AtomicBool,
    capture_calls: AtomicU64,
    capture_fail_after: AtomicU64,
    foreground_events: AtomicU64,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            ime_composing: AtomicBool::new(false),
            capture_fail_hard: AtomicBool::new(false),
            capture_calls: AtomicU64::new(0),
            capture_fail_after: AtomicU64::new(u64::MAX),
            foreground_events: AtomicU64::new(0),
        }
    }
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("mock sys poisoned")
    }
}

/// 脚本化平台实现。内部状态经 `Arc` 共享，可克隆给被测模块（守卫回调亦持同一份）。
#[derive(Clone, Default)]
pub struct MockSys {
    inner: Arc<Inner>,
}

impl MockSys {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- 脚本编制接口（测试侧） ----

    /// 设置虚拟桌面像素。
    pub fn set_screen(&self, screen: RgbaImage) {
        self.inner.state().screen = Some(screen);
    }

    /// 读取虚拟桌面（断言裁剪结果用）。
    pub fn screen(&self) -> Option<RgbaImage> {
        self.inner.state().screen.clone()
    }

    /// 登记/更新一个窗口。
    pub fn set_window(&self, hwnd: Hwnd, window: MockWindow) {
        self.inner.state().windows.insert(hwnd, window);
    }

    pub fn set_minimized(&self, hwnd: Hwnd, minimized: bool) {
        if let Some(w) = self.inner.state().windows.get_mut(&hwnd) {
            w.minimized = minimized;
        }
    }

    /// 直接设置前台窗口（不触发 watch 回调）。
    pub fn set_foreground(&self, info: Option<ForegroundInfo>) {
        self.inner.state().foreground = info;
    }

    /// 模拟前台切换：更新状态并投递给全部已注册回调。
    pub fn emit_foreground(&self, info: ForegroundInfo) {
        let cbs = {
            let mut st = self.inner.state();
            st.foreground = Some(info.clone());
            std::mem::take(&mut st.foreground_cbs)
        };
        // 回调在锁外调用，避免回调内再访问 MockSys 造成死锁
        for (_, cb) in &cbs {
            cb(info.clone());
        }
        let mut st = self.inner.state();
        st.foreground_cbs = cbs;
        self.inner.foreground_events.fetch_add(1, Ordering::SeqCst);
    }

    /// 投递一次按键：按注册顺序调用回调，任一 `Swallow` 即终止（与 RealSys 一致）。
    pub fn feed_key(&self, ev: KeyEvent) -> HookAction {
        let cbs = std::mem::take(&mut self.inner.state().key_cbs);
        let mut action = HookAction::Pass;
        for (_, cb) in &cbs {
            if cb(&ev) == HookAction::Swallow {
                action = HookAction::Swallow;
                break;
            }
        }
        self.inner.state().key_cbs = cbs;
        action
    }

    /// 钩子当前是否处于已安装状态（断言 `HookGuard::drop` 生效）。
    pub fn is_hooked(&self) -> bool {
        !self.inner.state().key_cbs.is_empty()
    }

    pub fn watch_installed(&self) -> bool {
        !self.inner.state().foreground_cbs.is_empty()
    }

    /// 已注册的键盘回调数量（多消费者场景的断言依据）。
    pub fn key_callback_count(&self) -> usize {
        self.inner.state().key_cbs.len()
    }

    pub fn foreground_callback_count(&self) -> usize {
        self.inner.state().foreground_cbs.len()
    }

    pub fn foreground_event_count(&self) -> u64 {
        self.inner.foreground_events.load(Ordering::SeqCst)
    }

    /// 让第 `n` 次之后的截图调用失败（0 = 立即失败），用于验证降级路径。
    pub fn fail_capture_after(&self, n: u64) {
        self.inner.capture_fail_after.store(n, Ordering::SeqCst);
    }

    /// 让截图永久失败。
    pub fn fail_capture(&self, fail: bool) {
        self.inner.capture_fail_hard.store(fail, Ordering::SeqCst);
    }

    pub fn capture_count(&self) -> u64 {
        self.inner.capture_calls.load(Ordering::SeqCst)
    }

    pub fn set_ime_composing(&self, composing: bool) {
        self.inner.ime_composing.store(composing, Ordering::SeqCst);
    }
}

impl SysApi for MockSys {
    fn foreground(&self) -> Option<ForegroundInfo> {
        self.inner.state().foreground.clone()
    }

    fn watch_foreground(&self, cb: ForegroundCallback) -> Result<WatchGuard, SysError> {
        let token = {
            let mut st = self.inner.state();
            let token = st.token();
            st.foreground_cbs.push((token, cb));
            token
        };
        let inner = Arc::clone(&self.inner);
        Ok(WatchGuard::new(move || {
            if let Ok(mut st) = inner.state.lock() {
                st.foreground_cbs.retain(|(t, _)| *t != token);
            }
        }))
    }

    fn window_rect(&self, hwnd: Hwnd) -> Result<(Rect, f32), SysError> {
        let st = self.inner.state();
        match st.windows.get(&hwnd) {
            Some(w) => Ok((to_physical(w.rect_dip, w.dpi_scale), w.dpi_scale)),
            None => Err(SysError::WindowUnavailable),
        }
    }

    fn is_window_minimized(&self, hwnd: Hwnd) -> bool {
        self.inner
            .state()
            .windows
            .get(&hwnd)
            .map(|w| w.minimized)
            .unwrap_or(true)
    }

    fn capture_region(&self, rect: Rect) -> Result<RgbaImage, SysError> {
        let n = self.inner.capture_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let hard_fail = self.inner.capture_fail_hard.load(Ordering::SeqCst);
        if hard_fail || n > self.inner.capture_fail_after.load(Ordering::SeqCst) {
            return Err(SysError::CaptureFailed("mock: capture disabled".into()));
        }
        if rect.is_empty() {
            return Err(SysError::WindowUnavailable);
        }
        let st = self.inner.state();
        let screen = st
            .screen
            .as_ref()
            .ok_or_else(|| SysError::CaptureFailed("mock: no screen configured".into()))?;

        let (sw, sh) = (screen.width() as i32, screen.height() as i32);
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = rect.right().min(sw);
        let y1 = rect.bottom().min(sh);
        if x1 <= x0 || y1 <= y0 {
            return Err(SysError::CaptureFailed(
                "mock: rect fully outside screen".into(),
            ));
        }
        let mut out = RgbaImage::new((x1 - x0) as u32, (y1 - y0) as u32);
        for y in y0..y1 {
            for x in x0..x1 {
                let px = *screen.get_pixel(x as u32, y as u32);
                out.put_pixel((x - x0) as u32, (y - y0) as u32, px);
            }
        }
        Ok(out)
    }

    fn install_keyboard_hook(&self, cb: KeyCallback) -> Result<HookGuard, SysError> {
        let token = {
            let mut st = self.inner.state();
            let token = st.token();
            st.key_cbs.push((token, cb));
            token
        };
        let inner = Arc::clone(&self.inner);
        Ok(HookGuard::new(move || {
            if let Ok(mut st) = inner.state.lock() {
                st.key_cbs.retain(|(t, _)| *t != token);
            }
        }))
    }

    fn ime_composing(&self) -> bool {
        self.inner.ime_composing.load(Ordering::SeqCst)
    }

    /// 模拟发现：前台进程名匹配即返回前台 hwnd（Mock 场景「目标窗口」总是
    /// 通过 `set_foreground` 注册的那一个；发现语义与前台判定天然一致）。
    /// 窗口必须存在（`set_window` 注册过）且未最小化。
    fn find_window_by_process(&self, process_name: &str) -> Option<Hwnd> {
        let st = self.inner.state();
        let fg = st.foreground.as_ref()?;
        if !fg.process_name.eq_ignore_ascii_case(process_name) {
            return None;
        }
        match st.windows.get(&fg.hwnd) {
            Some(w) if !w.minimized => Some(fg.hwnd),
            _ => None,
        }
    }
}
