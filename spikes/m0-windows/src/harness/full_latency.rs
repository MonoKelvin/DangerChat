//! M0-D 完整延迟：发送键按下 → 提示可见。
//!
//! 与 `e2e_latency` 的区别是这里**不省略 UI 渲染**：
//! 守卫判定抑制发送键（t0）→ 「正在检查」提示绘制完成 → 协调线程捕获 →
//! 结构定位 → OCR → 固定规则决策 → 结果提示绘制完成（t1）。
//!
//! 两项门槛（`docs/01` §15）：
//! - 「正在检查」提示在按键后 100 ms 内可见；
//! - 提示最终可见 p95 ≤ 800 ms。
//!
//! ## 为什么用合成事件驱动而不是物理按键
//!
//! 事件直接注入领域层 `GuardCore`，不调用 `SendInput` 等任何输入合成 API，
//! 因此红线扫描保持零白名单。这样做测得的仍是同一条链路：
//! `GuardCore::on_keyboard_event` 是钩子回调内实际执行的判定逻辑，
//! 而 t0 取自该判定返回「抑制」的时刻，与钩子回调内的取值点一致。
//!
//! 唯一未计入的是操作系统把按键投递到钩子回调的传输耗时。
//! 该段由 `m0 hook-latency` 在真实物理按键下单独测量（微秒量级），
//! 两者相加即为完整链路，报告中分别标注，不混为一谈。
//!
//! 目标是自有合成聊天窗口，不涉及任何第三方程序。

use std::time::Duration;

use crate::domain::guard::{
    ContextFingerprint, Decision, GuardCore, KeyAction, KeyEvent, TargetInstance,
};
use crate::domain::keys::{PhysicalKey, Shortcut, VK_RETURN};
use crate::harness::chat_window::{self, Scene, Theme};
use crate::harness::notice_window::{NoticeState, NoticeWindow};
use crate::harness::ocr_export::encode_roi_png;
use crate::harness::ocr_sidecar::OcrSidecar;
use crate::platform::capture::{capture_window_region, declare_dpi_awareness};
use crate::platform::clock::MonotonicClock;
use crate::platform::layout;
use crate::platform::window::snapshot_window;

/// 一次完整事务的延迟分解。全部以发送键判定为抑制的时刻为起点。
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

/// 主键盘 Enter。
const ENTER: PhysicalKey = PhysicalKey::new(VK_RETURN, 0x1C, false);

/// 运行完整链路测量。
///
/// 返回每轮的延迟分解。调用方负责统计与门槛判定。
pub fn run(rounds: usize) -> Result<Vec<TransactionTiming>, String> {
    declare_dpi_awareness();
    let clock = MonotonicClock::new();

    // 先启动 OCR 子进程：失败时不会遗留窗口。
    let mut sidecar = OcrSidecar::spawn()?;
    let notice = NoticeWindow::spawn()?;

    let Some(hwnd) = chat_window::create_window(Scene::sample(0, Theme::Light), 1280, 820) else {
        return Err("无法创建测试窗口".to_string());
    };
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(120));

    let result = collect(rounds, &notice, &mut sidecar, &clock, hwnd);

    drop(notice);
    drop(sidecar);
    chat_window::destroy_window(hwnd);
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(100));

    result
}

fn collect(
    rounds: usize,
    notice: &NoticeWindow,
    sidecar: &mut OcrSidecar,
    clock: &MonotonicClock,
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<Vec<TransactionTiming>, String> {
    let target = TargetInstance {
        root_hwnd: hwnd.0 as u64,
        pid: std::process::id(),
        process_start_time: 0,
    };

    let mut timings = Vec::with_capacity(rounds);

    for round in 0..rounds {
        // 每轮用全新守卫，确保状态与首次按键一致。
        let mut guard = GuardCore::new(Shortcut::Enter, 1);
        guard.arm(target, ContextFingerprint::default());

        // 轮换场景，覆盖不同草稿文本与主题。
        chat_window::set_scene(
            hwnd,
            Scene::sample(
                round,
                if round % 2 == 0 {
                    Theme::Light
                } else {
                    Theme::Dark
                },
            ),
        );
        if !chat_window::wait_for_repaint(hwnd, Duration::from_secs(2)) {
            return Err(format!("第 {round} 轮: 测试窗口未完成绘制"));
        }

        // t0：守卫判定抑制发送键的时刻，与钩子回调内的取值点一致。
        let t0 = clock.now_nanos();
        let decision = guard.on_keyboard_event(
            KeyEvent {
                key: ENTER,
                action: KeyAction::Down { repeat: false },
                injected: false,
            },
            t0,
        );
        if decision != Decision::Suppress {
            return Err(format!("第 {round} 轮: 首次发送键未被抑制（{decision:?}）"));
        }

        // 立即请求「正在检查」，这是 100 ms 反馈门槛的对象。
        notice.request(NoticeState::Checking);
        let checking_at = notice
            .wait_until_visible(NoticeState::Checking, clock, NOTICE_TIMEOUT_NANOS)
            .ok_or("「正在检查」提示未在超时内可见")?;

        guard.begin_capture();
        let analysis_started = clock.now_nanos();
        let outcome = analyze(sidecar, clock, hwnd);
        let analysis_nanos = clock.now_nanos().saturating_sub(analysis_started);
        guard.begin_analysis();

        let final_state = match &outcome {
            Ok(true) => NoticeState::Confirm,
            Ok(false) => NoticeState::Allow,
            Err(_) => NoticeState::Unavailable,
        };
        notice.request(final_state);
        let final_at = notice
            .wait_until_visible(final_state, clock, NOTICE_TIMEOUT_NANOS)
            .ok_or("结果提示未在超时内可见")?;

        notice.request(NoticeState::Hidden);

        let risky = outcome.map_err(|e| format!("第 {round} 轮分析失败: {e}"))?;
        // 有风险进入确认，无风险进入复核；两条路径都不在测量设施中签发许可。
        if risky {
            guard.require_review();
        } else {
            guard.begin_revalidation();
        }

        timings.push(TransactionTiming {
            feedback_nanos: checking_at.saturating_sub(t0),
            total_nanos: final_at.saturating_sub(t0),
            analysis_nanos,
        });
    }

    if timings.is_empty() {
        return Err("没有采集到任何事务样本".into());
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
