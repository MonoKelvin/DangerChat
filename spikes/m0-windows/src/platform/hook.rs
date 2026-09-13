//! 系统级低级键盘钩子。
//!
//! 关键约束（`docs/02_危信v1.0_技术架构与接口设计.md` §6.1、§7.1）：
//! - `WH_KEYBOARD_LL` 安装在本进程的专用线程，该线程独占消息循环。
//!   系统通过向该线程投递消息来调用回调，因此回调始终单线程执行，
//!   守卫状态无需互斥量即可保持 O(1) 快路径。
//! - 回调内不分配、不加锁、不写日志、不做 IPC、不截图、不调用 Python。
//! - `nCode < 0` 立即 `CallNextHookEx`。
//! - 回调超时会被系统静默移除（Windows 10 1709+ 上限按 1000 ms 计），
//!   因此内部目标远严于系统上限：p99 ≤ 0.25 ms。
//!
//! 本模块不存在、也不调用任何输入注入接口。

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL,
    WM_APP, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::domain::guard::{
    CancelReason, ContextFingerprint, Decision, GuardCore, GuardRequest, KeyAction, KeyEvent,
    PermitReason, TargetInstance, VerifiedSnapshot,
};
use crate::domain::keys::{PhysicalKey, Shortcut};
use crate::platform::clock::MonotonicClock;

/// 唤醒钩子线程去处理命令队列的自定义消息。
/// 目标是本进程自己的线程，不涉及任何外部窗口。
const WM_GUARD_COMMAND: u32 = WM_APP + 1;

/// 回调延迟直方图的预分配容量。超出后只更新统计量，不再分配。
const LATENCY_SAMPLES: usize = 1 << 20;

/// 钩子线程从协调侧接收的命令。
#[derive(Debug, Clone, Copy)]
pub enum GuardCommand {
    Arm {
        target: TargetInstance,
        fingerprint: ContextFingerprint,
    },
    Disarm(CancelReason),
    ContextChanged(ContextFingerprint),
    SetImeComposing(bool),
    SetHealth(bool),
    SetShortcut(Shortcut),
    BeginCapture,
    BeginAnalysis,
    BeginRevalidation,
    RequireReview,
    MarkUnavailable,
    ApproveOverride,
    PublishSnapshot(VerifiedSnapshot),
    IssuePermit(PermitReason),
    Cancel(CancelReason),
}

/// 回调侧统计。全部为原子量，回调内只做无锁累加。
#[derive(Debug, Default)]
pub struct HookMetrics {
    pub events_seen: AtomicU64,
    pub events_passed: AtomicU64,
    pub events_suppressed: AtomicU64,
    pub permits_consumed: AtomicU64,
    pub transactions_started: AtomicU64,
    pub rechecks_required: AtomicU64,
    /// 非目标状态下被抑制的事件数。必须恒为 0。
    pub false_suppressions: AtomicU64,
    pub callback_nanos_total: AtomicU64,
    pub callback_nanos_max: AtomicU64,
}

impl HookMetrics {
    fn record_callback(&self, nanos: u64) {
        self.callback_nanos_total
            .fetch_add(nanos, Ordering::Relaxed);
        self.callback_nanos_max.fetch_max(nanos, Ordering::Relaxed);
    }
}

/// 钩子线程与外部共享的控制块。
pub struct HookShared {
    pub metrics: HookMetrics,
    pub thread_id: AtomicU32,
    pub installed: AtomicBool,
    commands: Mutex<Vec<GuardCommand>>,
    /// 钩子线程向协调侧回传的最近一次请求。
    last_request: Mutex<Option<GuardRequest>>,
    /// 回调延迟样本，仅供离线统计 p99。
    latency: Mutex<Vec<u32>>,
}

impl HookShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            metrics: HookMetrics::default(),
            thread_id: AtomicU32::new(0),
            installed: AtomicBool::new(false),
            commands: Mutex::new(Vec::with_capacity(64)),
            last_request: Mutex::new(None),
            latency: Mutex::new(Vec::with_capacity(LATENCY_SAMPLES)),
        })
    }

    /// 从任意线程投递命令，并唤醒钩子线程的消息循环。
    pub fn send(&self, command: GuardCommand) {
        if let Ok(mut queue) = self.commands.lock() {
            queue.push(command);
        }
        let tid = self.thread_id.load(Ordering::Acquire);
        if tid != 0 {
            // SAFETY: 目标是本进程自己的钩子线程，不是任何外部窗口。
            unsafe {
                let _ = PostThreadMessageW(tid, WM_GUARD_COMMAND, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn take_request(&self) -> Option<GuardRequest> {
        self.last_request.lock().ok().and_then(|mut r| r.take())
    }

    /// 请求钩子线程退出消息循环。
    pub fn request_stop(&self) {
        let tid = self.thread_id.load(Ordering::Acquire);
        if tid != 0 {
            // SAFETY: 同上，目标是本进程自己的线程。
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// 返回排序后的延迟样本（纳秒）。
    pub fn latency_samples(&self) -> Vec<u32> {
        let mut samples = self.latency.lock().map(|v| v.clone()).unwrap_or_default();
        samples.sort_unstable();
        samples
    }
}

thread_local! {
    static HOOK_STATE: RefCell<Option<HookThreadState>> = const { RefCell::new(None) };
}

struct HookThreadState {
    guard: GuardCore,
    clock: MonotonicClock,
    shared: Arc<HookShared>,
    /// 预分配的延迟样本缓冲，回调内只写不扩容。
    latency: Vec<u32>,
}

/// 在当前线程安装钩子并运行消息循环，直到收到 `WM_QUIT`。
///
/// 必须在专用线程调用。
pub fn run_hook_thread(shared: Arc<HookShared>, shortcut: Shortcut, policy_version: u32) {
    let mut latency = Vec::new();
    latency.reserve_exact(LATENCY_SAMPLES);

    HOOK_STATE.with(|cell| {
        *cell.borrow_mut() = Some(HookThreadState {
            guard: GuardCore::new(shortcut, policy_version),
            clock: MonotonicClock::new(),
            shared: Arc::clone(&shared),
            latency,
        });
    });

    // SAFETY: 在本线程安装低级键盘钩子。hmod 传 None 对 WH_KEYBOARD_LL 有效，
    // 因为回调位于本进程内，系统不会把任何 DLL 注入其他进程。
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0) };
    let Ok(hook) = hook else {
        shared.installed.store(false, Ordering::Release);
        return;
    };

    // SAFETY: GetCurrentThreadId 无参数、无副作用。
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    shared.thread_id.store(tid, Ordering::Release);
    shared.installed.store(true, Ordering::Release);

    message_loop(&shared);

    // SAFETY: hook 由本线程安装，此处在同一线程卸载。
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
    shared.installed.store(false, Ordering::Release);

    // 把本线程采集的延迟样本交还给共享块。
    HOOK_STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().take() {
            if let Ok(mut sink) = shared.latency.lock() {
                *sink = state.latency;
            }
        }
    });
}

fn message_loop(shared: &Arc<HookShared>) {
    let mut msg = MSG::default();
    loop {
        // SAFETY: 标准消息循环，msg 为本地栈变量。
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        if msg.message == WM_GUARD_COMMAND {
            drain_commands(shared);
            continue;
        }
        // SAFETY: 转发本线程自身的消息。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    drain_commands(shared);
}

fn drain_commands(shared: &Arc<HookShared>) {
    let pending: Vec<GuardCommand> = match shared.commands.lock() {
        Ok(mut queue) => queue.drain(..).collect(),
        Err(_) => return,
    };
    if pending.is_empty() {
        return;
    }
    HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };
        let now = state.clock.now_nanos();
        for command in pending {
            apply_command(&mut state.guard, command, now);
        }
    });
}

fn apply_command(guard: &mut GuardCore, command: GuardCommand, now: u64) {
    match command {
        GuardCommand::Arm {
            target,
            fingerprint,
        } => guard.arm(target, fingerprint),
        GuardCommand::Disarm(reason) => guard.disarm(reason),
        GuardCommand::ContextChanged(fp) => guard.context_changed(fp),
        GuardCommand::SetImeComposing(v) => guard.set_ime_composing(v),
        GuardCommand::SetHealth(v) => guard.set_health(v),
        GuardCommand::SetShortcut(s) => guard.set_shortcut(s),
        GuardCommand::BeginCapture => guard.begin_capture(),
        GuardCommand::BeginAnalysis => guard.begin_analysis(),
        GuardCommand::BeginRevalidation => guard.begin_revalidation(),
        GuardCommand::RequireReview => guard.require_review(),
        GuardCommand::MarkUnavailable => guard.mark_unavailable(),
        GuardCommand::ApproveOverride => guard.approve_override(),
        GuardCommand::PublishSnapshot(s) => guard.publish_snapshot(s),
        GuardCommand::IssuePermit(reason) => {
            guard.issue_permit(reason, now);
        }
        GuardCommand::Cancel(reason) => {
            guard.cancel(reason);
        }
    }
}

/// 回调的可测量核心：判定 + 计数 + 延迟采样。
///
/// 抽成独立函数是为了让基准测试驱动与真实回调执行**同一段代码**，
/// 而不是另写一份近似实现去测一个不存在的路径。
fn process_event(state: &mut HookThreadState, event: KeyEvent, started: u64) -> bool {
    let decision = state.guard.on_keyboard_event(event, started);

    let metrics = &state.shared.metrics;
    metrics.events_seen.fetch_add(1, Ordering::Relaxed);
    match decision {
        Decision::Pass => {
            metrics.events_passed.fetch_add(1, Ordering::Relaxed);
        }
        Decision::Suppress => {
            metrics.events_suppressed.fetch_add(1, Ordering::Relaxed);
        }
        Decision::ConsumePermit => {
            metrics.events_passed.fetch_add(1, Ordering::Relaxed);
            metrics.permits_consumed.fetch_add(1, Ordering::Relaxed);
        }
    }

    if let Some(request) = state.guard.take_request() {
        match request {
            GuardRequest::StartTransaction { .. } => {
                metrics.transactions_started.fetch_add(1, Ordering::Relaxed);
            }
            GuardRequest::RecheckRequired { .. } => {
                metrics.rechecks_required.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        if let Ok(mut slot) = state.shared.last_request.try_lock() {
            *slot = Some(request);
        }
    }

    let elapsed = state.clock.now_nanos().saturating_sub(started);
    metrics.record_callback(elapsed);
    if state.latency.len() < state.latency.capacity() {
        state.latency.push(elapsed.min(u32::MAX as u64) as u32);
    }

    matches!(decision, Decision::Suppress)
}

/// 在不安装系统钩子的前提下，测量回调核心的处理耗时。
///
/// 用途：`docs/01` §15 的 p99 ≤ 0.25 ms 与单次最大 ≤ 2 ms 两项门槛。
/// 这里不调用 `SendInput` 等任何输入合成 API——事件直接构造后送入
/// 与真实回调相同的 `process_event`，因此源码中不存在输入注入接口。
///
/// 未计入的部分：操作系统把按键投递到回调的传输耗时，以及
/// 读取 `KBDLLHOOKSTRUCT` 的几条字段访问。前者不属于本进程可控范围，
/// 后者是常量时间的结构体字段读取。报告中据实标注。
pub fn benchmark_callback(
    shortcut: Shortcut,
    events: &[KeyEvent],
    clock: &MonotonicClock,
) -> Vec<u32> {
    let mut latency = Vec::new();
    latency.reserve_exact(events.len());

    let shared = HookShared::new();
    let mut state = HookThreadState {
        guard: GuardCore::new(shortcut, 1),
        clock: *clock,
        shared,
        latency,
    };

    for event in events {
        let started = state.clock.now_nanos();
        process_event(&mut state, *event, started);
    }

    let mut samples = state.latency;
    samples.sort_unstable();
    samples
}

/// 低级键盘钩子回调。必须常量时间返回。
unsafe extern "system" fn low_level_keyboard_proc(
    ncode: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if ncode < 0 {
        // SAFETY: 文档要求 nCode < 0 时原样转发。
        return unsafe { CallNextHookEx(None, ncode, wparam, lparam) };
    }

    let suppress = HOOK_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(state) = borrow.as_mut() else {
            return false;
        };

        let started = state.clock.now_nanos();

        // SAFETY: 对 HC_ACTION，系统保证 lparam 指向有效的 KBDLLHOOKSTRUCT。
        let info = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let message = wparam.0 as u32;

        let action = match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => KeyAction::Down { repeat: false },
            WM_KEYUP | WM_SYSKEYUP => KeyAction::Up,
            _ => return false,
        };

        let event = KeyEvent {
            key: PhysicalKey {
                vk: info.vkCode as u16,
                scan: info.scanCode as u16,
                extended: info.flags.0 & 0x01 != 0,
            },
            action,
            injected: info.flags.0 & LLKHF_INJECTED.0 != 0,
        };

        process_event(state, event, started)
    });

    if suppress {
        // 阻止按键继续传递。这是危信对系统输入的唯一动作：只做减法。
        return LRESULT(1);
    }
    // SAFETY: 未处理的事件必须继续钩子链，否则会影响其他程序。
    unsafe { CallNextHookEx(None, ncode, wparam, lparam) }
}
