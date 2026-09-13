//! M0-C 区域定位评测。
//!
//! 流程：创建自有测试聊天窗口 → 显示器级捕获 → DWM 裁剪 → 结构定位
//! → 与窗口自身的真值矩形比较 IoU。
//!
//! 覆盖多种窗口尺寸与浅/深主题。DPI 档位由运行环境决定，报告中如实标注。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

use crate::harness::chat_window::{self, Scene, Theme};
use crate::platform::capture::{capture_window_region, declare_dpi_awareness};
use crate::platform::clock::MonotonicClock;
use crate::platform::layout::{self, LayoutResult};
use crate::platform::window::snapshot_window;

/// 单个场景的评测结果。
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub label: String,
    pub theme: &'static str,
    pub client_width: i32,
    pub client_height: i32,
    pub dpi: u32,
    pub title_iou: f32,
    pub context_iou: f32,
    pub compose_iou: f32,
    pub confidence: f32,
    pub layout_signature: u64,
    pub capture_nanos: u64,
    pub detect_nanos: u64,
}

impl CaseResult {
    /// 关键 ROI 是否全部达标。标题与输入框是必需字段，阈值取 0.90。
    pub fn passed(&self) -> bool {
        self.title_iou >= 0.90 && self.compose_iou >= 0.90 && self.context_iou >= 0.85
    }

    pub fn min_iou(&self) -> f32 {
        self.title_iou.min(self.compose_iou).min(self.context_iou)
    }
}

/// 评测失败。任何一种都不得被当作定位成功。
#[derive(Debug)]
pub enum EvalError {
    WindowCreation,
    NotForeground,
    /// 超时未观察到目标场景的绘制完成，本次捕获无效。
    NotRendered,
    Capture(String),
    Layout(String),
}

fn pump_messages(duration: Duration) {
    let deadline = Instant::now() + duration;
    let mut msg = MSG::default();
    while Instant::now() < deadline {
        // SAFETY: 只处理本进程自己窗口的消息。
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 供同 crate 其他测试设施复用的消息泵。
pub fn pump_messages_public(duration: Duration) {
    pump_messages(duration)
}

/// 把客户区真值矩形换算到捕获帧坐标。
///
/// 捕获帧以 DWM 扩展边界为原点，客户区原点由 `ClientToScreen` 精确给出，
/// 两者相减即为帧内偏移，不做边框厚度估算。
fn client_to_frame(gt: RECT, offset_x: i32, offset_y: i32) -> RECT {
    RECT {
        left: gt.left + offset_x,
        top: gt.top + offset_y,
        right: gt.right + offset_x,
        bottom: gt.bottom + offset_y,
    }
}

/// 对一个已显示的测试窗口执行一次捕获与定位。
fn evaluate_window(
    hwnd: HWND,
    label: &str,
    theme: Theme,
    clock: &MonotonicClock,
) -> Result<CaseResult, EvalError> {
    // 必须确认场景已真正绘制并呈现，否则屏幕级捕获会读到半成品帧。
    if !chat_window::wait_for_repaint(hwnd, Duration::from_secs(2)) {
        return Err(EvalError::NotRendered);
    }

    let snapshot = snapshot_window(hwnd).ok_or(EvalError::WindowCreation)?;
    if snapshot.instance.root_hwnd != hwnd.0 as u64 || !snapshot.visible || snapshot.minimized {
        return Err(EvalError::WindowCreation);
    }

    let frame = capture_window_region(snapshot.instance.root_hwnd, snapshot.client, clock)
        .map_err(|e| EvalError::Capture(format!("{e:?}")))?;

    let detect_started = clock.now_nanos();
    let detected: LayoutResult = layout::detect(
        &frame.pixels,
        frame.width as i32,
        frame.height as i32,
        frame.stride as usize,
    )
    .map_err(|e| EvalError::Layout(format!("{e:?}")))?;
    let detect_nanos = clock.now_nanos().saturating_sub(detect_started);

    let client = chat_window::client_rect(hwnd).ok_or(EvalError::NotForeground)?;
    let client_origin =
        chat_window::client_origin_on_screen(hwnd).ok_or(EvalError::NotForeground)?;
    let gt = chat_window::layout_for(client);
    let client_w = client.right - client.left;
    let client_h = client.bottom - client.top;

    // 帧原点是裁剪区左上角（桌面坐标），客户区原点同为桌面坐标，相减得到帧内偏移。
    let offset_x = client_origin.x - frame.origin.x;
    let offset_y = client_origin.y - frame.origin.y;
    let expect = |r: RECT| client_to_frame(r, offset_x, offset_y);

    Ok(CaseResult {
        label: label.to_string(),
        theme: match theme {
            Theme::Light => "light",
            Theme::Dark => "dark",
        },
        client_width: client_w,
        client_height: client_h,
        dpi: snapshot.dpi,
        title_iou: layout::iou(detected.chat_title, expect(gt.chat_title)),
        context_iou: layout::iou(detected.message_context, expect(gt.message_context)),
        compose_iou: layout::iou(detected.compose_input, expect(gt.compose_input)),
        confidence: detected.confidence,
        layout_signature: detected.layout_signature,
        capture_nanos: frame.capture_nanos,
        detect_nanos,
    })
}

/// 跑完整评测矩阵：多种窗口尺寸 × 浅/深主题 × 多个合成场景。
pub fn run_matrix() -> Vec<Result<CaseResult, EvalError>> {
    declare_dpi_awareness();
    let clock = MonotonicClock::new();
    let mut results = Vec::new();

    const SIZES: [(i32, i32); 4] = [(1000, 720), (1280, 820), (1440, 900), (860, 640)];

    for theme in [Theme::Light, Theme::Dark] {
        for (idx, (w, h)) in SIZES.iter().enumerate() {
            let scene = Scene::sample(idx, theme);
            let label = format!("{}x{}#{}", w, h, scene.title);

            let Some(hwnd) = chat_window::create_window(scene, *w, *h) else {
                results.push(Err(EvalError::WindowCreation));
                continue;
            };
            // 等待窗口映射到屏幕；实际绘制完成由 evaluate_window 显式确认。
            pump_messages(Duration::from_millis(120));
            results.push(evaluate_window(hwnd, &label, theme, &clock));
            chat_window::destroy_window(hwnd);
            pump_messages(Duration::from_millis(120));
        }
    }

    results
}
