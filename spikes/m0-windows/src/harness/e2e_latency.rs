//! M0-D 合成分析链延迟测量（不等同于物理按键到提示可见）。
//!
//! 链路：函数入口（t0）→ 显示器捕获 → 结构定位 → 内存 PNG →
//! 保活 Python OCR 进程识别标题与草稿 → 固定规则决策（t1）。
//!
//! 说明：
//! - 不计入物理按键、协调排队和 UI 渲染，不能据此判定 M0-D 通过。
//! - 完整 M0-D 门槛仍需在 M0 自有交互窗口中复核，不能推迟到 M1。
//! - OCR 通过保活子进程避免每事务冷启动模型。

use std::time::Duration;

use crate::harness::chat_window::{self, Scene, Theme};
use crate::harness::ocr_export::encode_roi_png;
use crate::harness::ocr_sidecar::OcrSidecar;
use crate::platform::capture::{capture_window_region, declare_dpi_awareness};
use crate::platform::clock::MonotonicClock;
use crate::platform::layout;
use crate::platform::window::snapshot_window;

/// 一轮完整的分析链。
///
/// 说明：延迟测量不要求测试窗口处于前台（自动化环境中终端会不断重申焦点），
/// 而是依赖置顶显示 + 定位成功作为"捕获到正确像素"的校验。
/// 真实产品的前台触发语义由领域层单测与 M0-A 回放覆盖。
fn analyze_once(
    hwnd: windows::Win32::Foundation::HWND,
    sidecar: &mut OcrSidecar,
    clock: &MonotonicClock,
) -> Result<u64, String> {
    let t0 = clock.now_nanos();

    let snapshot = snapshot_window(hwnd).ok_or("无法读取测试窗口元数据")?;
    if snapshot.instance.root_hwnd != hwnd.0 as u64 || !snapshot.visible || snapshot.minimized {
        return Err("测试窗口不可捕获".to_string());
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

    let title_png =
        encode_roi_png(&frame, inset(detected.chat_title, 4, 4)).map_err(|e| e.to_string())?;
    let draft_png =
        encode_roi_png(&frame, inset(detected.compose_input, 4, 4)).map_err(|e| e.to_string())?;
    let title = sidecar.recognize(title_png)?;
    let draft = sidecar.recognize(draft_png)?;
    // 本测试场景的标题、草稿均非空。空识别意味着链路不完整，不能计为成功样本。
    if title.trim().is_empty() || draft.trim().is_empty() {
        return Err("标题或草稿未识别到文字".into());
    }
    let risky = ["离谱", "受不了", "辞职"].iter().any(|w| draft.contains(w));
    let _decision = if risky { "CONFIRM" } else { "ALLOW_PERMIT" };
    Ok(clock.now_nanos().saturating_sub(t0))
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

/// 运行端到端延迟测量。
pub fn run(rounds: usize) -> Result<Vec<u64>, String> {
    declare_dpi_awareness();
    let clock = MonotonicClock::new();

    // 在创建窗口前完成子进程握手，启动失败时不会遗留测试窗口。
    let mut sidecar = OcrSidecar::spawn()?;

    // 测试窗口保持前台，场景轮换以覆盖不同文本。
    let scene = Scene::sample(0, Theme::Light);
    let Some(hwnd) = chat_window::create_window(scene, 1280, 820) else {
        return Err("无法创建测试窗口".to_string());
    };
    // 等待窗口映射到屏幕；每轮的绘制完成在循环内显式确认。
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(120));

    let mut durations = Vec::with_capacity(rounds);
    let mut errors: Vec<String> = Vec::new();
    for i in 0..rounds {
        // 轮换场景文本，模拟不同草稿。
        chat_window::set_scene(
            hwnd,
            Scene::sample(
                i,
                if i % 2 == 0 {
                    Theme::Light
                } else {
                    Theme::Dark
                },
            ),
        );
        // 绘制等待属于测试脚手架，必须在 t0 之前完成，不计入分析链延迟。
        if !chat_window::wait_for_repaint(hwnd, Duration::from_secs(2)) {
            errors.push(format!("第 {i} 轮: 测试窗口未完成绘制"));
            break;
        }

        match analyze_once(hwnd, &mut sidecar, &clock) {
            Ok(nanos) => durations.push(nanos),
            Err(e) => {
                errors.push(format!("第 {i} 轮: {e}"));
                break;
            }
        }
    }

    drop(sidecar);
    chat_window::destroy_window(hwnd);
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(100));

    if !errors.is_empty() {
        let sample: Vec<String> = errors.iter().take(3).cloned().collect();
        return Err(format!(
            "{}/{} 轮分析失败，示例：{}",
            errors.len(),
            rounds,
            sample.join("；")
        ));
    }
    Ok(durations)
}
