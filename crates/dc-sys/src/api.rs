//! 平台层对外契约（设计文档 §5.2）。

use image::RgbaImage;

/// 窗口句柄（裸 `HWND` 的 thin wrapper，避免 Win32 类型泄漏到业务代码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hwnd(pub isize);

impl Hwnd {
    pub fn is_null(self) -> bool {
        self.0 == 0
    }

    pub fn raw(self) -> isize {
        self.0
    }
}

impl std::fmt::Display for Hwnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hwnd(0x{:X})", self.0)
    }
}

/// 屏幕坐标系矩形（物理像素）。`w`/`h` 恒非负。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w as i32)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h as i32)
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// 以左上角为锚点按缩放比换算（`ROUND` 而非截断，避免逐次缩放累计丢 1px）。
    pub fn scaled(&self, scale: f32) -> Rect {
        to_physical(*self, scale)
    }

    /// 两矩形是否几何相同（布局复用校验用，§2.3）。
    pub fn same_geometry(&self, other: &Rect) -> bool {
        self == other
    }
}

/// DIP → 物理像素换算。
///
/// 左上角与右下角**分别**取整后再算宽高：这样 `(x, y, w, h)` 与 `(x, y, x+w, y+h)`
/// 两种表达在同一缩放下得到一致的矩形，不会出现右/下边界 1px 漂移。
pub fn to_physical(rect: Rect, dpi_scale: f32) -> Rect {
    if rect.is_empty() {
        return rect;
    }
    let scale = if dpi_scale.is_finite() && dpi_scale > 0.0 {
        dpi_scale
    } else {
        1.0
    };
    let x = (rect.x as f32 * scale).round() as i32;
    let y = (rect.y as f32 * scale).round() as i32;
    let r = ((rect.x + rect.w as i32) as f32 * scale).round() as i32;
    let b = ((rect.y + rect.h as i32) as f32 * scale).round() as i32;
    Rect {
        x,
        y,
        w: (r - x).max(0) as u32,
        h: (b - y).max(0) as u32,
    }
}

/// Windows 报告的系统 DPI → 缩放比（96 DPI = 1.0）。
pub fn dpi_scale_from_dpi(dpi: u32) -> f32 {
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// 前台窗口信息。`process_name` 为可执行文件名（含扩展名），如 `WeChat.exe`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub hwnd: Hwnd,
    pub pid: u32,
    pub process_name: String,
}

/// 低级键盘钩子收到的一次按键事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// 虚拟键码。
    pub vk: u16,
    pub scan_code: u32,
    /// true = 按下（WM_KEYDOWN/WM_SYSKEYDOWN），false = 抬起。
    pub is_key_down: bool,
    /// 是否由其他程序注入（`LLKHF_INJECTED`）。用于日志排障，**不**用于对抗行为。
    pub is_injected: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 输入法消费掉的键在低级钩子里出现的虚拟键码（§5.2 主信号）。
pub const VK_PROCESSKEY: u16 = 0xE5;

impl KeyEvent {
    /// 该事件是否为「被输入法消费的键」——输入法正在组合（拼音候选）的零成本信号。
    pub fn is_ime_consumed(&self) -> bool {
        self.vk == VK_PROCESSKEY
    }
}

/// 钩子回调的裁决结果（**只减不加**：只有「放行」与「吞掉」两种，不存在注入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    /// 原样传递（调用 `CallNextHookEx`）。
    Pass,
    /// 吞掉该按键，不向任何程序传递。
    Swallow,
}

/// 键盘钩子回调。**必须在 < 1ms 内返回**（§5.3），只允许原子操作与 `try_send`。
pub type KeyCallback = Box<dyn Fn(&KeyEvent) -> HookAction + Send + 'static>;

/// 前台切换回调。§5.3 的 ForegroundTracker 在回调内只做原子置位与去抖。
pub type ForegroundCallback = Box<dyn Fn(ForegroundInfo) + Send + 'static>;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SysError {
    #[error("target window unavailable")]
    WindowUnavailable,
    #[error("capture failed: {0}")]
    CaptureFailed(String),
    #[error("hook install failed: {0}")]
    HookInstallFailed(String),
    #[error("sys thread not running")]
    ThreadDown,
    #[error("win32 error: {0}")]
    Win32(String),
    #[error("operation not supported on this platform")]
    Unsupported,
}

/// 键盘钩子句柄，`Drop` 即卸载钩子。
pub struct HookGuard {
    teardown: Option<Box<dyn FnOnce() + Send>>,
}

impl HookGuard {
    pub fn new(teardown: impl FnOnce() + Send + 'static) -> Self {
        Self {
            teardown: Some(Box::new(teardown)),
        }
    }

    /// 取出拆卸动作但不执行（供需要显式控制的场景，极少使用）。
    pub fn disarm(mut self) -> Option<Box<dyn FnOnce() + Send>> {
        self.teardown.take()
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        if let Some(f) = self.teardown.take() {
            f();
        }
    }
}

impl std::fmt::Debug for HookGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookGuard")
            .field("armed", &self.teardown.is_some())
            .finish()
    }
}

/// 前台事件监听句柄，`Drop` 即取消订阅。
pub struct WatchGuard {
    teardown: Option<Box<dyn FnOnce() + Send>>,
}

impl WatchGuard {
    pub fn new(teardown: impl FnOnce() + Send + 'static) -> Self {
        Self {
            teardown: Some(Box::new(teardown)),
        }
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        if let Some(f) = self.teardown.take() {
            f();
        }
    }
}

impl std::fmt::Debug for WatchGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchGuard")
            .field("armed", &self.teardown.is_some())
            .finish()
    }
}

/// 平台能力契约。所有方法不得因目标程序状态而 panic；错误一律经 [`SysError`] 返回。
pub trait SysApi: Send + Sync {
    /// 当前前台窗口信息；无前台窗口（如锁屏）返回 `None`。
    fn foreground(&self) -> Option<ForegroundInfo>;

    /// 订阅前台切换事件（`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`，事件驱动、零轮询）。
    ///
    /// 与设计文档 §5.2 的签名差异：这里额外返回 `Result`，使订阅失败可被上层感知并降级；
    /// 失败时返回 `Err` 而非静默的空守卫。
    fn watch_foreground(&self, cb: ForegroundCallback) -> Result<WatchGuard, SysError>;

    /// 目标窗口矩形（**物理像素**，已完成 DPI 换算）与缩放比。
    fn window_rect(&self, hwnd: Hwnd) -> Result<(Rect, f32), SysError>;

    /// 窗口是否最小化（`IsIconic`）。遮挡不做检测（FR-CAP-03）。
    fn is_window_minimized(&self, hwnd: Hwnd) -> bool;

    /// 屏幕像素拷贝指定矩形（屏幕 DC BitBlt，ADR-12）。
    fn capture_region(&self, rect: Rect) -> Result<RgbaImage, SysError>;

    /// 安装全局低级键盘钩子（`WH_KEYBOARD_LL`）。
    fn install_keyboard_hook(&self, cb: KeyCallback) -> Result<HookGuard, SysError>;

    /// 输入法是否处于组合态（`VK_PROCESSKEY` + `EVENT_OBJECT_IME_*`，FR-SRC-09）。
    fn ime_composing(&self) -> bool;
}
