//! AlertBus 泵线程：后端事件 → Tauri 事件 + dc-alert 窗口显示（FR-BRG-01）。
//!
//! - `AlertMessage::Show(Verdict)` → `alert://blocked` 事件 + 弹窗定位显示 + 统计 +1；
//! - `AlertMessage::Action(AlertAction)`（钩子在 Cooldown 期代转的数字键）
//!   → `alert://action` 事件（弹窗高亮对应按钮的视觉反馈；真正执行仍走
//!   `intercept.apply_alert_action`——**由泵直接执行**，事件只做 UI 提示）；
//! - 状态/统计心跳：状态在迁移时由 commands/window_watch 主动 emit；
//!   统计心跳由本线程每 2s emit 一次（guard://stats）。

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use dc_pipeline::intercept::{AlertAction, AlertMessage};
use dc_sys::{Hwnd, SysApi};

use crate::dto::{AlertPayload, StatsPayload};
use crate::events;
use crate::state::AppState;

/// 起泵线程（非阻塞，返回 JoinHandle 供壳测试用）。
pub fn spawn(app: AppHandle) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("bridge-pump".into())
        .spawn(move || pump_loop(app))
        .expect("bridge-pump spawn")
}

fn pump_loop(app: AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let alerts = state.intercept.alerts_arc();
    let mut last_stats_emit = std::time::Instant::now() - Duration::from_secs(10);

    loop {
        // 双通道 select：AlertBus 事件（100ms 超时）+ 统计心跳节流
        match alerts.recv_timeout(Duration::from_millis(100)) {
            Some(AlertMessage::Show(verdict)) => {
                state.stats.record_block();
                let countdown = *state.countdown_secs.lock().unwrap_or_else(|e| e.into_inner());
                let payload = AlertPayload::from_verdict(&verdict, countdown);
                let t0 = std::time::Instant::now();
                if let Err(e) = show_alert(&app, &payload) {
                    state.stats.record_alert_failed();
                    tracing::warn!(error = %e, "弹窗显示失败（fail-open：无阻断动作）");
                }
                let _ = t0.elapsed();
                // 事件在 show 之后发：前端 AlertRoot listen 后填充 DOM（预渲染骨架）
                if let Err(e) = app.emit(events::EVENT_ALERT, &payload) {
                    tracing::warn!(error = %e, "alert 事件发送失败");
                }
            }
            Some(AlertMessage::Action(a)) => {
                // 钩子代转的数字键：真正执行动作（与弹窗按钮同一条路径）
                state.intercept.apply_alert_action(a);
                // UI 提示事件（前端可闪高亮；失败不影响已执行的动作）
                let name = match a {
                    AlertAction::Allow => "allow",
                    AlertAction::Cancel => "cancel",
                    AlertAction::Edit => "edit",
                    AlertAction::Snooze => "snooze",
                };
                let _ = app.emit("alert://action", name);
            }
            None => {}
        }

        if last_stats_emit.elapsed() >= Duration::from_secs(2) {
            last_stats_emit = std::time::Instant::now();
            let _ = app.emit(events::EVENT_STATS, collect_stats(&state));
        }
    }
}

/// 弹窗显示：定位到目标窗口顶部居中 + show（不 set_focus，FR-BRG-04）。
fn show_alert(app: &AppHandle, payload: &AlertPayload) -> Result<(), String> {
    let state = app.state::<Arc<AppState>>();
    let win = app
        .get_webview_window("alert")
        .ok_or("alert 窗口未预建")?;

    // 定位：目标窗口顶部居中（拿不到 rect 就居屏）
    let target: Option<Hwnd> = state
        .guard
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|g| g.target_hwnd());
    if let Some(hwnd) = target {
        if let Ok((rect, _)) = SysApi::window_rect(state.sys.as_ref(), hwnd) {
            let (w, h) = (360.0f64, 260.0);
            // 目标窗口正中（遮挡聊天流中部，避开底部输入框与顶部标题栏）
            let x = rect.x as f64 + (rect.w as f64 - w) / 2.0;
            let y = rect.y as f64 + (rect.h as f64 - h) / 2.0;
            let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
            let _ = win.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
        }
    }
    let _ = payload; // DOM 填充走事件（emit 在 show 后）
    win.show().map_err(|e| e.to_string())?;
    // 不 set_focus：弹窗 focusable:false，键盘焦点留在目标窗口
    Ok(())
}

/// 组装统计载荷（各 Stage 最近耗时，无消息原文）。
fn collect_stats(state: &AppState) -> StatsPayload {
    let mut p = StatsPayload {
        today_blocked: state.stats.blocked(),
        alert_failed: state.stats.alert_failed(),
        ..Default::default()
    };
    let guard = state.guard.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(g) = guard.as_ref() {
        use dc_pipeline::contract::Module;
        let core = g.core();
        let avg_ms = |m: &dyn Module| {
            let s = m.metrics();
            if s.invocations == 0 {
                None
            } else {
                Some(s.avg_duration_ns() as f64 / 1e6)
            }
        };
        p.last_capture_ms = avg_ms(&*core.capture);
        p.last_layout_ms = avg_ms(&*core.layout);
        p.last_ocr_ms = avg_ms(&*core.ocr);
        p.last_sem_ms = avg_ms(&*core.sem);
    }
    p
}
