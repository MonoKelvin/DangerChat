//! M0-D 完整延迟：物理按键 → 提示可见。
//!
//! 与 `e2e_latency` 的区别是这里**不省略任何一段**：
//! 真实低级键盘钩子抑制物理按键（t0）→ 协调线程捕获 → 结构定位 →
//! OCR → 固定规则决策 → 提示窗口完成绘制（t1）。
//!
//! 两项门槛（`docs/01` §15）：
//! - 「正在检查」提示在按键后 100 ms 内可见；
//! - 提示最终可见 p95 ≤ 800 ms。
//!
//! 需要真实物理按键：本程序不合成任何输入，也不模拟按键事件。
//! 目标是自有合成聊天窗口，不涉及任何第三方程序。

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::guard::{ContextFingerprint, GuardRequest, TargetInstance};
use crate::domain::keys::Shortcut;
use crate::harness::chat_window::{self, Scene, Theme};
use crate::harness::notice_window::{NoticeState, NoticeWindow};
use crate::harness::ocr_export::encode_roi_png;
use crate::harness::ocr_sidecar::OcrSidecar;
use crate::platform::capture::{capture_window_region, declare_dpi_awareness};
use crate::platform::clock::MonotonicClock;
use crate::platform::hook::{run_hook_thread, GuardCommand, HookShared};
use crate::platform::layout;
use crate::platform::window::snapshot_window;

/// 一次完整事务的延迟分解。全部以物理按键时刻为起点。
#[derive(Debug, Clone, Copy)]
pub struct TransactionTiming {
    /// 按键 → 「正在检查」提示可见。
    pub feedback_nanos: u64,
    /// 按键 → 最终结果提示可见。
    pub total_nanos: u64,
    /// 其中的分析耗时（捕获 + 定位 + OCR）。
    pub analysis_nanos: u64,
}

/// 提示窗最长等待时间：超过即判该轮失败，不用请求时刻冒充可见时刻。
const NOTICE_TIMEOUT_NANOS: u64 = 3_000_000_000;

/// 运行完整链路测量，直到收集满 `rounds` 个样本或超时。
///
/// 返回每轮的延迟分解。调用方负责统计与门槛判定。
pub fn run(rounds: usize, budget: Duration) -> Result<Vec<TransactionTiming>, String> {
    declare_dpi_awareness();
    let clock = MonotonicClock::new();

    // 先启动 OCR 子进程：失败时不会遗留窗口与钩子。
    let mut sidecar = OcrSidecar::spawn()?;

    let notice = NoticeWindow::spawn()?;

    let Some(hwnd) = chat_window::create_window(Scene::sample(0, Theme::Light), 1280, 820) else {
        return Err("无法创建测试窗口".to_string());
    };
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(120));

    let shared = HookShared::new();
    let hook_thread = {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("dc-hook".into())
            .spawn(move || run_hook_thread(shared, Shortcut::Enter, 1))
            .map_err(|e| format!("无法创建钩子线程: {e}"))?
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !shared.installed.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !shared.installed.load(Ordering::Acquire) {
        return Err("钩子安装失败".into());
    }

    // arm 到自有测试窗口：只有该窗口前台时才拦截 Enter。
    let target = TargetInstance {
        root_hwnd: hwnd.0 as u64,
        pid: std::process::id(),
        process_start_time: 0,
    };
    shared.send(GuardCommand::Arm {
        target,
        fingerprint: ContextFingerprint::default(),
    });

    let result = collect(
        rounds,
        budget,
        &shared,
        &notice,
        &mut sidecar,
        &clock,
        hwnd,
        target,
    );

    shared.request_stop();
    let _ = hook_thread.join();
    drop(notice);
    drop(sidecar);
    chat_window::destroy_window(hwnd);
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(100));

    result
}

#[allow(clippy::too_many_arguments)]
fn collect(
    rounds: usize,
    budget: Duration,
    shared: &Arc<HookShared>,
    notice: &NoticeWindow,
    sidecar: &mut OcrSidecar,
    clock: &MonotonicClock,
    hwnd: windows::Win32::Foundation::HWND,
    target: TargetInstance,
) -> Result<Vec<TransactionTiming>, String> {
    let mut timings = Vec::with_capacity(rounds);
    let give_up = Instant::now() + budget;

    while timings.len() < rounds {
        if Instant::now() >= give_up {
            break;
        }
        // 等待钩子报告一次新事务（即用户按下了被抑制的发送键）。
        let Some(GuardRequest::StartTransaction { .. }) = shared.take_request() else {
            crate::harness::layout_eval::pump_messages_public(Duration::from_millis(8));
            continue;
        };
        let t0 = shared.last_transaction_nanos.load(Ordering::Acquire);
        if t0 == 0 {
            return Err("缺少物理按键时间戳".into());
        }

        // 立即请求「正在检查」，这是 100 ms 反馈门槛的对象。
        notice.request(NoticeState::Checking);
        let checking_at = notice
            .wait_until_visible(NoticeState::Checking, clock, NOTICE_TIMEOUT_NANOS)
            .ok_or("「正在检查」提示未在超时内可见")?;

        shared.send(GuardCommand::BeginCapture);
        let analysis_started = clock.now_nanos();
        let outcome = analyze(sidecar, clock, hwnd);
        let analysis_nanos = clock.now_nanos().saturating_sub(analysis_started);

        let final_state = match &outcome {
            Ok(true) => NoticeState::Confirm,
            Ok(false) => NoticeState::Allow,
            Err(_) => NoticeState::Unavailable,
        };
        shared.send(GuardCommand::BeginAnalysis);
        notice.request(final_state);
        let final_at = notice
            .wait_until_visible(final_state, clock, NOTICE_TIMEOUT_NANOS)
            .ok_or("结果提示未在超时内可见")?;

        // 本轮结束：撤销事务并回到可再次触发的状态。
        // 测量设施不签发许可——放行路径由 M0-A 回放覆盖。
        shared.send(GuardCommand::Cancel(
            crate::domain::guard::CancelReason::UserCancelled,
        ));
        shared.send(GuardCommand::Arm {
            target,
            fingerprint: ContextFingerprint::default(),
        });
        notice.request(NoticeState::Hidden);

        if let Err(e) = outcome {
            return Err(format!("第 {} 轮分析失败: {e}", timings.len()));
        }

        timings.push(TransactionTiming {
            feedback_nanos: checking_at.saturating_sub(t0),
            total_nanos: final_at.saturating_sub(t0),
            analysis_nanos,
        });
    }

    if timings.is_empty() {
        return Err("没有采集到任何事务样本：请在测试窗口前台时按 Enter".into());
    }
    Ok(timings)
}

/// 执行一次分析，返回是否命中风险词。
fn analyze(
    sidecar: &mut OcrSidecar,
    clock: &MonotonicClock,
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<bool, String> {
    let snapshot = snapshot_window(hwnd).ok_or("无法读取测试窗口元数据")?;
    if !snapshot.visible || snapshot.minimized {
        return Err("测试窗口不可捕获".into());
    }
    let frame = capture_window_region(snapshot.instance.root_hwnd, snapshot.client, clock)
        .map_err(|e| format!("捕获失败: {e:?}"))?;
    let detected = layout::detect(
        &frame.pixels,
        frame.width as i32,
        frame.height as i32,
        frame.stride as usize,
    )
    .map_err(|e| format!("定位失败: {e:?}"))?;

    let draft_png =
        encode_roi_png(&frame, inset(detected.compose_input, 4, 4)).map_err(|e| e.to_string())?;
    let draft = sidecar.recognize(draft_png)?;
    if draft.trim().is_empty() {
        return Err("草稿未识别到文字".into());
    }
    Ok(["离谱", "受不了", "辞职"].iter().any(|w| draft.contains(w)))
}

fn inset(
    rect: windows::Win32::Foundation::RECT,
    dx: i32,
    dy: i32,
) -> windows::Win32::Foundation::RECT {
    windows::Win32::Foundation::RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right - dx,
        bottom: rect.bottom - dy,
    }
}
