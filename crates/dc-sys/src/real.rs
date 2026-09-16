//! `RealSys`：真实桌面的 [`SysApi`] 实现。**全仓库唯一直接调用 Win32 的代码。**
//!
//! 结构：
//!
//! ```text
//! 调用线程                       sys-thread（消息泵）
//! ────────                       ────────────────────
//! RealSys::capture_region  ──►   BitBlt（无需消息循环，就地调用）
//! RealSys::install_keyboard_hook ─► SetWindowsHookExW(WH_KEYBOARD_LL)
//! RealSys::watch_foreground      ─► SetWinEventHook(EVENT_SYSTEM_FOREGROUND)
//!                                  + EVENT_OBJECT_IME_*（同一线程，IME 状态修正）
//!                                PeekMessage 泵消息 → 钩子/事件回调在 sys-thread 上执行
//! ```
//!
//! 为什么必须有独立线程：`WH_KEYBOARD_LL` 的回调由系统投递到**安装钩子的那个线程**，
//! 该线程必须持续泵消息，否则回调不会被调用（并会导致系统输入超时）。sys-thread 用
//! `PeekMessage` + 自动重置事件唤醒，空闲时不占 CPU。

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, LRESULT, RECT, SYSTEMTIME, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, QueryFullProcessImageNameW, SetEvent, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    GetAwarenessFromDpiAwarenessContext, GetDpiForWindow, GetThreadDpiAwarenessContext,
    GetWindowDpiAwarenessContext, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_PER_MONITOR_AWARE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, MsgWaitForMultipleObjectsEx, PeekMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, EVENT_OBJECT_IME_CHANGE, EVENT_OBJECT_IME_HIDE, EVENT_OBJECT_IME_SHOW,
    EVENT_SYSTEM_FOREGROUND, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE,
    QS_ALLINPUT, WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_KEYDOWN,
    WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::api::{
    dpi_scale_from_dpi, ForegroundCallback, ForegroundInfo, HookAction, HookGuard, Hwnd,
    KeyCallback, KeyEvent, Rect, SysApi, SysError, WatchGuard, VK_PROCESSKEY,
};

// ---------------------------------------------------------------------------
// 共享状态：键盘/前台回调只能经 thread-local 与原子量访问（§3.2 无锁、无重入）
// ---------------------------------------------------------------------------

thread_local! {
    /// 键盘回调注册表：`(token, callback)`，**按注册顺序**依次调用，
    /// 任一回调返回 `Swallow` 即终止本次分发（后续回调收不到该事件）。
    ///
    /// 为什么是「一个 OS 钩子 + 回调表」而不是「一次安装一个钩子」：
    /// 低级钩子回调只能经 thread-local 取到；若每次安装都覆盖单槽回调，
    /// 先安装者会被**静默顶替**（钩子过程还在被系统调用，但跑的是后装者的逻辑）。
    static KEY_CBS: std::cell::RefCell<Vec<(u64, KeyCallback)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// 前台回调注册表（同上；前台事件没有「吞掉」语义，全部回调都会收到）。
    static FG_CBS: std::cell::RefCell<Vec<(u64, ForegroundCallback)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// IME 组合态：`VK_PROCESSKEY` 为主信号（最近一次消费键的单调毫秒），
/// `EVENT_OBJECT_IME_SHOW/HIDE` 修正组合开始/结束边界（FR-SRC-09、§5.2）。
static IME_LAST_PROCESSKEY_MS: AtomicU64 = AtomicU64::new(0);
static KEY_PROC_INVOKED: AtomicU64 = AtomicU64::new(0);
static FG_PROC_INVOKED: AtomicU64 = AtomicU64::new(0);
static IME_SHOWN: AtomicBool = AtomicBool::new(false);

/// 距离最近一次「被输入法消费的键」多久仍视为组合中。
const IME_PROCESSKEY_TTL_MS: u64 = 1_000;

fn mono_ms() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn ime_composing_now() -> bool {
    if IME_SHOWN.load(Ordering::SeqCst) {
        return true;
    }
    let last = IME_LAST_PROCESSKEY_MS.load(Ordering::SeqCst);
    last != 0 && mono_ms().saturating_sub(last) < IME_PROCESSKEY_TTL_MS
}

// ---------------------------------------------------------------------------
// sys-thread：钩子安装与消息泵
// ---------------------------------------------------------------------------

enum SysReq {
    /// 注册键盘回调；返回本次注册的 token（卸载时用它精确摘除）。
    RegisterKeyboard(KeyCallback, Sender<Result<u64, String>>),
    UnregisterKeyboard(u64, Sender<()>),
    RegisterForeground(ForegroundCallback, Sender<Result<u64, String>>),
    UnregisterForeground(u64, Sender<()>),
    Quit(Sender<()>),
}

#[derive(Default)]
struct ThreadState {
    /// 进程内唯一的键盘钩子（只有回调表清空时才真正卸载）。
    keyboard: Option<windows::Win32::UI::WindowsAndMessaging::HHOOK>,
    foreground: Option<HWINEVENTHOOK>,
    ime: Option<HWINEVENTHOOK>,
    next_token: u64,
}

impl ThreadState {
    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }
}

impl ThreadState {
    fn unhook_all(&mut self) {
        unsafe {
            if let Some(h) = self.keyboard.take() {
                let _ = UnhookWindowsHookEx(h);
            }
            if let Some(h) = self.foreground.take() {
                let _ = UnhookWinEvent(h);
            }
            if let Some(h) = self.ime.take() {
                let _ = UnhookWinEvent(h);
            }
        }
    }
}

struct SysThread {
    tx: Sender<SysReq>,
    /// 自动重置事件句柄（以 `isize` 持有：`HANDLE` 不是 `Send`）。
    event: isize,
    shutdown: Arc<AtomicBool>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl SysThread {
    fn spawn() -> Result<Arc<Self>, SysError> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
            .map_err(|e| SysError::HookInstallFailed(format!("CreateEventW: {e}")))?;
        let event_raw = event.0 as isize;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_thread = Arc::clone(&shutdown);
        let join = std::thread::Builder::new()
            .name("dc-sys-hook".to_string())
            .spawn(move || sys_thread_main(rx, event_raw, shutdown_thread))
            .map_err(|e| SysError::HookInstallFailed(format!("spawn sys thread: {e}")))?;

        Ok(Arc::new(Self {
            tx,
            event: event_raw,
            shutdown,
            join: Mutex::new(Some(join)),
        }))
    }

    /// 投递请求并唤醒 sys-thread（事件为自动重置，仅用于唤醒，请求本体走 channel）。
    fn request(&self, req: SysReq) {
        if self.tx.send(req).is_err() {
            tracing::warn!("sys thread 已退出，请求被丢弃");
            return;
        }
        unsafe {
            let _ = SetEvent(HANDLE(self.event as *mut core::ffi::c_void));
        }
    }
}

impl Drop for SysThread {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.request(SysReq::Quit(tx));
        let _ = rx.recv_timeout(Duration::from_secs(2));
        if let Some(join) = self.join.lock().expect("sys join poisoned").take() {
            let _ = join.join();
        }
        unsafe {
            let _ = CloseHandle(HANDLE(self.event as *mut core::ffi::c_void));
        }
    }
}

fn sys_thread_main(ctrl: Receiver<SysReq>, event: isize, shutdown: Arc<AtomicBool>) {
    let mut state = ThreadState::default();
    let mut msg = MSG::default();
    let mut last_heartbeat = std::time::Instant::now();

    while !shutdown.load(Ordering::SeqCst) {
        if last_heartbeat.elapsed() >= std::time::Duration::from_secs(5) {
            last_heartbeat = std::time::Instant::now();
            tracing::info!(
                key_invoked = KEY_PROC_INVOKED.load(Ordering::SeqCst),
                fg_invoked = FG_PROC_INVOKED.load(Ordering::SeqCst),
                keyboard_hook = state.keyboard.is_some(),
                foreground_hook = state.foreground.is_some(),
                "sys-thread 心跳"
            );
        }
        // 1) 处理安装/卸载请求
        while let Ok(req) = ctrl.try_recv() {
            handle_request(req, &mut state);
        }
        // 2) 泵消息：低级键盘钩子回调与 WinEvent 回调都在这里被触发
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // 3) 阻塞等待（输入到达即返回；事件用于安装/退出请求唤醒）
            MsgWaitForMultipleObjectsEx(
                Some(&[HANDLE(event as *mut core::ffi::c_void)]),
                200,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
        }
    }
    state.unhook_all();
}

fn handle_request(req: SysReq, state: &mut ThreadState) {
    match req {
        SysReq::RegisterKeyboard(cb, reply) => {
            let token = state.token();
            KEY_CBS.with(|cbs| cbs.borrow_mut().push((token, cb)));
            // 只在首个回调注册时才装 OS 钩子：装两个钩子会让同一事件被投递两次
            if state.keyboard.is_none() {
                match install_keyboard() {
                    Ok(hook) => {
                        state.keyboard = Some(hook);
                        // IME 组合态边界修正（EVENT_OBJECT_IME_SHOW..CHANGE，连续区间一次订阅）。
                        // 失败不致命：主信号 VK_PROCESSKEY 仍在低级钩子里生效（§5.2）。
                        match install_win_event(
                            EVENT_OBJECT_IME_SHOW,
                            EVENT_OBJECT_IME_CHANGE,
                            Some(ime_proc),
                        ) {
                            Ok(ime) => state.ime = Some(ime),
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "IME 事件订阅失败，退化为仅 VK_PROCESSKEY 判定"
                                )
                            }
                        }
                        let _ = reply.send(Ok(token));
                    }
                    Err(e) => {
                        // 安装失败：回滚刚注册的回调，不留永远收不到事件的条目
                        KEY_CBS.with(|cbs| cbs.borrow_mut().retain(|(t, _)| *t != token));
                        let _ = reply.send(Err(e));
                    }
                }
            } else {
                let _ = reply.send(Ok(token));
            }
        }
        SysReq::UnregisterKeyboard(token, reply) => {
            KEY_CBS.with(|cbs| cbs.borrow_mut().retain(|(t, _)| *t != token));
            let empty = KEY_CBS.with(|cbs| cbs.borrow().is_empty());
            if empty {
                if let Some(h) = state.keyboard.take() {
                    unsafe {
                        let _ = UnhookWindowsHookEx(h);
                    }
                }
                if let Some(h) = state.ime.take() {
                    unsafe {
                        let _ = UnhookWinEvent(h);
                    }
                }
            }
            let _ = reply.send(());
        }
        SysReq::RegisterForeground(cb, reply) => {
            let token = state.token();
            FG_CBS.with(|cbs| cbs.borrow_mut().push((token, cb)));
            if state.foreground.is_none() {
                match install_win_event(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    Some(foreground_proc),
                ) {
                    Ok(hook) => {
                        state.foreground = Some(hook);
                        let _ = reply.send(Ok(token));
                    }
                    Err(e) => {
                        FG_CBS.with(|cbs| cbs.borrow_mut().retain(|(t, _)| *t != token));
                        let _ = reply.send(Err(e));
                    }
                }
            } else {
                let _ = reply.send(Ok(token));
            }
        }
        SysReq::UnregisterForeground(token, reply) => {
            FG_CBS.with(|cbs| cbs.borrow_mut().retain(|(t, _)| *t != token));
            let empty = FG_CBS.with(|cbs| cbs.borrow().is_empty());
            if empty {
                if let Some(h) = state.foreground.take() {
                    unsafe {
                        let _ = UnhookWinEvent(h);
                    }
                }
            }
            let _ = reply.send(());
        }
        SysReq::Quit(reply) => {
            let _ = reply.send(());
        }
    }
}

fn install_keyboard() -> Result<windows::Win32::UI::WindowsAndMessaging::HHOOK, String> {
    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) }
        .map_err(|e| format!("SetWindowsHookExW(WH_KEYBOARD_LL): {e}"))
}

fn install_win_event(
    event_min: u32,
    event_max: u32,
    proc: windows::Win32::UI::Accessibility::WINEVENTPROC,
) -> Result<HWINEVENTHOOK, String> {
    let hook = unsafe {
        SetWinEventHook(
            event_min,
            event_max,
            None,
            proc,
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.is_invalid() {
        Err(format!("SetWinEventHook({event_min:#06X}) 失败"))
    } else {
        Ok(hook)
    }
}

// ---------------------------------------------------------------------------
// 钩子回调
// ---------------------------------------------------------------------------

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    KEY_PROC_INVOKED.fetch_add(1, Ordering::SeqCst);
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let msg = wparam.0 as u32;
    let is_key_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let is_key_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    if !is_key_down && !is_key_up {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let vk = kb.vkCode as u16;

    if vk == VK_PROCESSKEY && is_key_down {
        // 输入法正在组合：主信号（§5.2）
        IME_LAST_PROCESSKEY_MS.store(mono_ms(), Ordering::SeqCst);
    }

    let ev = KeyEvent {
        vk,
        scan_code: kb.scanCode,
        is_key_down,
        is_injected: kb.flags.contains(LLKHF_INJECTED),
        ctrl: key_pressed(VK_CONTROL.0),
        alt: key_pressed(VK_MENU.0),
        shift: key_pressed(VK_SHIFT.0),
    };

    // 按注册顺序分发；任一回调吞键即终止分发（后续回调收不到该事件）
    let swallow = KEY_CBS.with(|cbs| {
        cbs.borrow()
            .iter()
            .any(|(_, cb)| cb(&ev) == HookAction::Swallow)
    });

    if swallow {
        LRESULT(1)
    } else {
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

fn key_pressed(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

unsafe extern "system" fn foreground_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    FG_PROC_INVOKED.fetch_add(1, Ordering::SeqCst);
    if event != EVENT_SYSTEM_FOREGROUND {
        return;
    }
    let target = Hwnd(hwnd.0 as isize);
    let Some(info) = foreground_info(target) else {
        return;
    };
    FG_CBS.with(|cbs| {
        for (_, cb) in cbs.borrow().iter() {
            cb(info.clone());
        }
    });
}

unsafe extern "system" fn ime_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    match event {
        EVENT_OBJECT_IME_SHOW | EVENT_OBJECT_IME_CHANGE => {
            IME_SHOWN.store(true, Ordering::SeqCst);
        }
        EVENT_OBJECT_IME_HIDE => {
            IME_SHOWN.store(false, Ordering::SeqCst);
        }
        _ => {}
    }
}

fn foreground_info(hwnd: Hwnd) -> Option<ForegroundInfo> {
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(HWND(hwnd.0 as *mut core::ffi::c_void), Some(&mut pid));
    }
    if pid == 0 {
        return None;
    }
    Some(ForegroundInfo {
        hwnd,
        pid,
        process_name: process_name_of(pid).unwrap_or_default(),
    })
}

fn process_name_of(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok || len == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Some(
            Path::new(&full)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&full)
                .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// RealSys
// ---------------------------------------------------------------------------

/// 确认进程为 per-monitor DPI 感知；已由宿主预先设置时视为成功。
///
/// **为什么不能只看 `SetProcessDpiAwarenessContext` 的返回值**：该 API 在
/// 「进程 DPI 感知级别已设置过」时一律返回 FALSE（`GetLastError` =
/// `ERROR_ACCESS_DENIED`），而**不管已设级别是否正是我们要的那一档**。
/// 宿主是 Tauri：`tauri::Builder` 创建窗口时会令 WebView2 设置进程 DPI 感知，
/// 于是 `RealSys::new()`（在 `bootstrap` 中、Builder 之后调用）必然撞上
/// 「已设置」分支。若把这种情况当失败，就是**把期望状态误报成故障** ——
/// 日志每次启动都刷一条无意义的 warn，且 `is_dpi_aware()` 会骗过调用方。
///
/// 故改为**读实际状态**判定：能查询到 per-monitor（V2 或 V1）即为满足。
///
/// 坐标系一致性（本函数存在的唯一目的，FR-CAP-04）：
/// - per-monitor 感知 → `GetWindowRect` 返回物理像素，屏幕 DC 亦在物理像素系，
///   两者可直接同用（§5.4「禁止二次缩放」的前提）；
/// - 感知级别为 unaware / system-aware → 上述坐标可能被 DWM 虚拟化，截图必然错位。
///
/// 降级策略（§2.1 宁漏勿阻）：即便拿到不可用的感知级别，也**不阻断启动** ——
/// 只记 warn 让上层可观测，行为交由后续截图自身的失败路径（Recoverable → fail-open）。
fn ensure_per_monitor_dpi_aware() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };

    // 1) 进程是否**已经**是 per-monitor 感知（含 V1/V2 两档，均已满足坐标一致性）。
    //    有前台窗口时按窗口查（对混合模式进程更准），否则退化为查当前线程。
    let current = unsafe {
        if hwnd.is_invalid() {
            GetThreadDpiAwarenessContext()
        } else {
            GetWindowDpiAwarenessContext(hwnd)
        }
    };
    let current_awareness = unsafe { GetAwarenessFromDpiAwarenessContext(current) };
    if current_awareness == DPI_AWARENESS_PER_MONITOR_AWARE {
        tracing::debug!("进程已是 per-monitor DPI 感知（宿主已设置）");
        return true;
    }

    // 2) 尚未设置：尝试提升到 V2。此调用仅在「本进程首次设置」时才可能成功。
    match unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
        Ok(()) => {
            tracing::info!("已切换为 per-monitor DPI 感知（V2）");
            true
        }
        Err(_) => {
            // 3) 设置被拒（已被更早的清单/API 固定为 unaware 或 system-aware）。
            //    复核实际级别，避免把「本来就对」误判为失败。
            let err = unsafe { windows::Win32::Foundation::GetLastError() };
            tracing::warn!(
                awareness = current_awareness.0,
                error = ?err,
                "DPI 感知级别非 per-monitor 且无法提升，高分屏下截图坐标可能偏移（不阻断启动）"
            );
            false
        }
    }
}

/// 真实平台实现。进程内单实例使用（内部持有 sys-thread 与位图复用缓冲）。
pub struct RealSys {
    thread: Mutex<Option<Arc<SysThread>>>,
    /// 截图 DIB 缓冲，按窗口矩形尺寸分配后复用（§5.4，替代全屏双缓冲）
    scratch: Mutex<Vec<u8>>,
    dpi_aware: bool,
}

impl RealSys {
    pub fn new() -> Self {
        let dpi_aware = ensure_per_monitor_dpi_aware();
        Self {
            thread: Mutex::new(None),
            scratch: Mutex::new(Vec::new()),
            dpi_aware,
        }
    }

    /// 进程是否成功切换为 per-monitor DPI 感知。
    pub fn is_dpi_aware(&self) -> bool {
        self.dpi_aware
    }

    fn sys_thread(&self) -> Result<Arc<SysThread>, SysError> {
        let mut guard = self.thread.lock().expect("real sys poisoned");
        if let Some(t) = guard.as_ref() {
            return Ok(Arc::clone(t));
        }
        let spawned = SysThread::spawn()?;
        *guard = Some(Arc::clone(&spawned));
        Ok(spawned)
    }

    /// 请求 + 等待回执（安装/卸载是低频动作，可同步等待；失败返回错误由上层降级）。
    /// 返回本次注册的 token，卸载时用它**精确摘除自己的回调**（不会误伤其他消费者）。
    fn call(
        &self,
        build: impl FnOnce(Sender<Result<u64, String>>) -> SysReq,
    ) -> Result<u64, SysError> {
        let thread = self.sys_thread()?;
        let (tx, rx) = crossbeam_channel::bounded(1);
        thread.request(build(tx));
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(token)) => Ok(token),
            Ok(Err(msg)) => Err(SysError::HookInstallFailed(msg)),
            Err(_) => Err(SysError::ThreadDown),
        }
    }

    fn call_void(&self, build: impl FnOnce(Sender<()>) -> SysReq) {
        let Ok(thread) = self.sys_thread() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        thread.request(build(tx));
        let _ = rx.recv_timeout(Duration::from_secs(3));
    }
}

impl Default for RealSys {
    fn default() -> Self {
        Self::new()
    }
}

impl SysApi for RealSys {
    fn foreground(&self) -> Option<ForegroundInfo> {
        let hwnd = Hwnd(unsafe { GetForegroundWindow() }.0 as isize);
        foreground_info(hwnd)
    }

    fn watch_foreground(&self, cb: ForegroundCallback) -> Result<WatchGuard, SysError> {
        let token = self.call(|reply| SysReq::RegisterForeground(cb, reply))?;
        let this = self as *const RealSys as usize;
        Ok(WatchGuard::new(move || {
            // 安全：RealSys 生命周期长于守卫（进程级单实例），且 call_void 只做线程通信
            if let Some(sys) = unsafe { (this as *const RealSys).as_ref() } {
                sys.call_void(|reply| SysReq::UnregisterForeground(token, reply));
            }
        }))
    }

    fn window_rect(&self, hwnd: Hwnd) -> Result<(Rect, f32), SysError> {
        if hwnd.is_null() {
            return Err(SysError::WindowUnavailable);
        }
        let mut raw = RECT::default();
        unsafe { GetWindowRect(HWND(hwnd.0 as *mut core::ffi::c_void), &mut raw) }
            .map_err(|e| SysError::Win32(format!("GetWindowRect: {e}")))?;
        let w = (raw.right - raw.left).max(0) as u32;
        let h = (raw.bottom - raw.top).max(0) as u32;
        let dpi = unsafe { GetDpiForWindow(HWND(hwnd.0 as *mut core::ffi::c_void)) };
        // 进程已是 per-monitor DPI 感知，GetWindowRect 给出的就是物理像素，
        // 此处不再做二次换算（§5.4 明确禁止二次缩放）。
        Ok((Rect::new(raw.left, raw.top, w, h), dpi_scale_from_dpi(dpi)))
    }

    fn is_window_minimized(&self, hwnd: Hwnd) -> bool {
        if hwnd.is_null() {
            return true;
        }
        unsafe { IsIconic(HWND(hwnd.0 as *mut core::ffi::c_void)).as_bool() }
    }

    fn capture_region(&self, rect: Rect) -> Result<RgbaImage, SysError> {
        if rect.is_empty() {
            return Err(SysError::WindowUnavailable);
        }
        let w = rect.w as i32;
        let h = rect.h as i32;
        let len = (rect.w as usize) * (rect.h as usize) * 4;

        let mut scratch = self.scratch.lock().expect("scratch poisoned");
        if scratch.len() < len {
            scratch.resize(len, 0);
        }
        scratch[..len].fill(0);

        unsafe {
            let screen_dc = GetDC(None);
            if screen_dc.is_invalid() {
                return Err(SysError::CaptureFailed("GetDC(NULL) 失败".into()));
            }
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            if mem_dc.is_invalid() {
                let _ = ReleaseDC(None, screen_dc);
                return Err(SysError::CaptureFailed("CreateCompatibleDC 失败".into()));
            }
            let bitmap = CreateCompatibleBitmap(screen_dc, w, h);
            if bitmap.is_invalid() {
                let _ = DeleteDC(mem_dc);
                let _ = ReleaseDC(None, screen_dc);
                return Err(SysError::CaptureFailed(
                    "CreateCompatibleBitmap 失败".into(),
                ));
            }
            let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

            // 仅拷贝目标窗口矩形（ADR-12）：屏幕 DC 作为源，不分配全屏缓冲
            let blit = BitBlt(mem_dc, 0, 0, w, h, Some(screen_dc), rect.x, rect.y, SRCCOPY);

            let mut wrote = 0i32;
            if blit.is_ok() {
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader.biSize =
                    std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w;
                // 负高度 = top-down，行序与图像一致，省一次翻转
                bmi.bmiHeader.biHeight = -h;
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB.0;
                wrote = GetDIBits(
                    mem_dc,
                    bitmap,
                    0,
                    h as u32,
                    Some(scratch.as_mut_ptr() as *mut core::ffi::c_void),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
            }

            SelectObject(mem_dc, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);

            if blit.is_err() {
                return Err(SysError::CaptureFailed(format!("BitBlt: {:?}", blit.err())));
            }
            if wrote == 0 {
                return Err(SysError::CaptureFailed("GetDIBits 未写入任何行".into()));
            }
        }

        // BGRA(DIB, 内存小端) → RGBA
        let mut out = RgbaImage::new(rect.w, rect.h);
        for y in 0..rect.h {
            for x in 0..rect.w {
                let i = ((y as usize) * (rect.w as usize) + x as usize) * 4;
                out.put_pixel(
                    x,
                    y,
                    image::Rgba([scratch[i + 2], scratch[i + 1], scratch[i], 255]),
                );
            }
        }
        Ok(out)
    }

    fn install_keyboard_hook(&self, cb: KeyCallback) -> Result<HookGuard, SysError> {
        let token = self.call(|reply| SysReq::RegisterKeyboard(cb, reply))?;
        let this = self as *const RealSys as usize;
        Ok(HookGuard::new(move || {
            if let Some(sys) = unsafe { (this as *const RealSys).as_ref() } {
                sys.call_void(|reply| SysReq::UnregisterKeyboard(token, reply));
            }
        }))
    }

    fn ime_composing(&self) -> bool {
        ime_composing_now()
    }

    fn find_window_by_process(&self, process_name: &str) -> Option<Hwnd> {
        find_window_by_process_impl(process_name)
    }
}

/// EnumWindows 遍历顶层窗口，返回第一个「可见 + 非工具窗 + 非最小化 + 进程名匹配」的。
/// 进程名比较不区分大小写。
fn find_window_by_process_impl(process_name: &str) -> Option<Hwnd> {
    use windows::core::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, IsIconic, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    struct Ctx {
        want: String,
        found: Option<Hwnd>,
    }
    // EnumWindows 回调约定：返回 TRUE 继续遍历
    unsafe extern "system" fn on_wnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };
        if ctx.found.is_some() {
            return true.into();
        }
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return true.into();
            }
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if ex & WS_EX_TOOLWINDOW.0 as i32 != 0 {
                return true.into();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return true.into();
            }
            match process_name_of(pid) {
                Some(name) if name.eq_ignore_ascii_case(&ctx.want) => {
                    ctx.found = Some(Hwnd(hwnd.0 as isize));
                    false.into() // 停止遍历
                }
                _ => true.into(),
            }
        }
    }

    let mut ctx = Ctx {
        want: process_name.to_string(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(on_wnd), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found
}

/// 本地时区相对 UTC 的偏移（分钟，东为正）。
///
/// 用「本地时间 − UTC 时间」相减得到，避免依赖 `GetTimeZoneInformation` 的偏差语义；
/// 跨 DST 切换瞬间可能有 60 分钟误差，仅用于日志文件按本地日期滚动，可接受。
pub fn local_utc_offset_minutes() -> i32 {
    let (utc, local) = unsafe { (GetSystemTime(), GetLocalTime()) };
    let to_min = |s: &SYSTEMTIME| -> i64 {
        dc_core::time::days_from_civil(s.wYear as i64, s.wMonth as u32, s.wDay as u32) * 1440
            + s.wHour as i64 * 60
            + s.wMinute as i64
    };
    (to_min(&local) - to_min(&utc)) as i32
}

/// 本地日期（年/月/日），供 bridge 的按日统计滚动用。
pub fn local_ymd() -> (u32, u32, u32) {
    let t = unsafe { GetLocalTime() };
    (t.wYear as u32, t.wMonth as u32, t.wDay as u32)
}
