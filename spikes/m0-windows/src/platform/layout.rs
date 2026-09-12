//! 结构化区域定位。
//!
//! 依据 `docs/02_危信v1.0_技术架构与接口设计.md` §10.2：
//! 优先使用分隔线、边缘与几何关系，不把语言相关的按钮文字当作锚点，
//! 也不使用目标检测模型。未知或多义结构一律判为不确定，不得产出“安全”结论。
//!
//! 输入是已按客户区裁剪的目标窗口像素（BGRA8，自上而下）。
//! 使用客户区而非整窗，可避免把窗口自身的标题栏与边框误判为界面结构。

use windows::Win32::Foundation::RECT;

/// 定位失败原因。任何一种都必须进入 RX，不得静默放行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    /// 帧尺寸低于最小支持客户区。
    TooSmall,
    /// 找不到可信的竖直分隔线（侧栏边界）。
    NoSidebarBoundary,
    /// 找不到可信的输入区分隔线。
    NoComposeSeparator,
    /// 找到多条同样可信的候选，结构含义不唯一。
    Ambiguous,
    /// 整体置信度低于阈值。
    LowConfidence,
}

/// 定位结果。坐标为帧内像素坐标。
#[derive(Debug, Clone, Copy)]
pub struct LayoutResult {
    pub chat_title: RECT,
    pub message_context: RECT,
    pub compose_input: RECT,
    pub sidebar_width: i32,
    pub separator_y: i32,
    pub title_bottom: i32,
    /// 不含任何文本内容的结构签名，用于选择 LayoutProfile 与兼容性判定。
    pub layout_signature: u64,
    pub confidence: f32,
}

/// 定位所需的最小客户区，与 LayoutProfile 的 `min_client_size` 对应。
const MIN_WIDTH: i32 = 640;
const MIN_HEIGHT: i32 = 480;

/// 判定为分隔线所需的最小行/列一致性比例。
const LINE_CONSISTENCY: f32 = 0.80;

/// 灰度化。BGRA8 输入。
fn gray_at(pixels: &[u8], stride: usize, x: i32, y: i32) -> u8 {
    let idx = y as usize * stride + x as usize * 4;
    let b = pixels[idx] as u32;
    let g = pixels[idx + 1] as u32;
    let r = pixels[idx + 2] as u32;
    ((r * 77 + g * 150 + b * 29) >> 8) as u8
}

/// 一行中相邻像素的平均纵向梯度。分隔线表现为局部峰值。
fn horizontal_edge_strength(pixels: &[u8], stride: usize, width: i32, y: i32, x0: i32) -> f32 {
    if y <= 0 {
        return 0.0;
    }
    let mut diff_sum = 0u32;
    let mut count = 0u32;
    let mut x = x0;
    while x < width {
        let above = gray_at(pixels, stride, x, y - 1) as i32;
        let here = gray_at(pixels, stride, x, y) as i32;
        diff_sum += (here - above).unsigned_abs();
        count += 1;
        x += 1;
    }
    if count == 0 {
        0.0
    } else {
        diff_sum as f32 / count as f32
    }
}

/// 一列中相邻像素的平均横向梯度。
fn vertical_edge_strength(pixels: &[u8], stride: usize, height: i32, x: i32) -> f32 {
    if x <= 0 {
        return 0.0;
    }
    let mut diff_sum = 0u32;
    let mut count = 0u32;
    let mut y = 0;
    while y < height {
        let left = gray_at(pixels, stride, x - 1, y) as i32;
        let here = gray_at(pixels, stride, x, y) as i32;
        diff_sum += (here - left).unsigned_abs();
        count += 1;
        y += 1;
    }
    if count == 0 {
        0.0
    } else {
        diff_sum as f32 / count as f32
    }
}

/// 该行在给定水平范围内是否足够一致（真正的分隔线整行同色）。
fn row_consistency(pixels: &[u8], stride: usize, width: i32, y: i32, x0: i32) -> f32 {
    if y <= 0 {
        return 0.0;
    }
    let mut consistent = 0u32;
    let mut total = 0u32;
    let mut x = x0;
    while x < width {
        let above = gray_at(pixels, stride, x, y - 1) as i32;
        let here = gray_at(pixels, stride, x, y) as i32;
        if (here - above).abs() >= 6 {
            consistent += 1;
        }
        total += 1;
        x += 1;
    }
    if total == 0 {
        0.0
    } else {
        consistent as f32 / total as f32
    }
}

fn column_consistency(pixels: &[u8], stride: usize, height: i32, x: i32) -> f32 {
    if x <= 0 {
        return 0.0;
    }
    let mut consistent = 0u32;
    let mut total = 0u32;
    let mut y = 0;
    while y < height {
        let left = gray_at(pixels, stride, x - 1, y) as i32;
        let here = gray_at(pixels, stride, x, y) as i32;
        if (here - left).abs() >= 6 {
            consistent += 1;
        }
        total += 1;
        y += 1;
    }
    if total == 0 {
        0.0
    } else {
        consistent as f32 / total as f32
    }
}

/// 在给定纵向范围内找出最强且唯一的水平分隔线。
fn find_horizontal_separator(
    pixels: &[u8],
    stride: usize,
    width: i32,
    y_from: i32,
    y_to: i32,
    x0: i32,
) -> Result<(i32, f32), LayoutError> {
    let mut best = (0i32, 0f32);
    let mut runner_up = 0f32;

    for y in y_from.max(1)..y_to {
        let consistency = row_consistency(pixels, stride, width, y, x0);
        if consistency < LINE_CONSISTENCY {
            continue;
        }
        let strength = horizontal_edge_strength(pixels, stride, width, y, x0) * consistency;
        if strength > best.1 {
            // 与已有最优相距很近的行属于同一条线的抗锯齿边缘，不算竞争候选。
            if (y - best.0).abs() > 3 {
                runner_up = best.1;
            }
            best = (y, strength);
        } else if strength > runner_up && (y - best.0).abs() > 3 {
            runner_up = strength;
        }
    }

    if best.1 <= 0.0 {
        return Err(LayoutError::NoComposeSeparator);
    }
    // 次优候选过于接近最优时，结构含义不唯一。
    if runner_up > best.1 * 0.92 {
        return Err(LayoutError::Ambiguous);
    }
    Ok(best)
}

/// 找出侧栏与主区之间的竖直边界。
fn find_sidebar_boundary(
    pixels: &[u8],
    stride: usize,
    width: i32,
    height: i32,
) -> Result<(i32, f32), LayoutError> {
    // 侧栏宽度的合理范围，避免把窗口边框当成边界。
    let x_from = (width as f32 * 0.12) as i32;
    let x_to = (width as f32 * 0.45) as i32;

    let mut best = (0i32, 0f32);
    let mut runner_up = 0f32;

    for x in x_from.max(1)..x_to {
        let consistency = column_consistency(pixels, stride, height, x);
        if consistency < LINE_CONSISTENCY {
            continue;
        }
        let strength = vertical_edge_strength(pixels, stride, height, x) * consistency;
        if strength > best.1 {
            if (x - best.0).abs() > 3 {
                runner_up = best.1;
            }
            best = (x, strength);
        } else if strength > runner_up && (x - best.0).abs() > 3 {
            runner_up = strength;
        }
    }

    if best.1 <= 0.0 {
        return Err(LayoutError::NoSidebarBoundary);
    }
    if runner_up > best.1 * 0.92 {
        return Err(LayoutError::Ambiguous);
    }
    Ok(best)
}

/// 结构签名：只由几何比例与 DPI 档位构成，不含任何文本或像素内容。
fn compute_signature(width: i32, height: i32, sidebar: i32, title_h: i32, separator_y: i32) -> u64 {
    // 量化到千分之一，容忍亚像素抖动但能区分不同布局版本。
    let q = |value: i32, total: i32| -> u64 {
        if total <= 0 {
            0
        } else {
            ((value as f64 / total as f64) * 1000.0).round() as u64
        }
    };
    let aspect = q(width, height);
    let side = q(sidebar, width);
    let title = q(title_h, height);
    let compose = q(height - separator_y, height);

    let mut hash = 0xcbf29ce484222325u64;
    for part in [aspect, side, title, compose] {
        hash ^= part;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 对已裁剪的目标窗口帧执行结构定位。
pub fn detect(
    pixels: &[u8],
    width: i32,
    height: i32,
    stride: usize,
) -> Result<LayoutResult, LayoutError> {
    if width < MIN_WIDTH || height < MIN_HEIGHT {
        return Err(LayoutError::TooSmall);
    }
    if pixels.len() < stride * height as usize {
        return Err(LayoutError::TooSmall);
    }

    let (sidebar, sidebar_strength) = find_sidebar_boundary(pixels, stride, width, height)?;

    // 输入区分隔线：只在窗口下半部分搜索，且限定在主区水平范围内。
    let (separator_y, sep_strength) = find_horizontal_separator(
        pixels,
        stride,
        width,
        (height as f32 * 0.55) as i32,
        (height as f32 * 0.95) as i32,
        sidebar + 2,
    )?;

    // 标题栏下沿：在窗口上半部分搜索。
    let (title_bottom, title_strength) = find_horizontal_separator(
        pixels,
        stride,
        width,
        (height as f32 * 0.03) as i32,
        (height as f32 * 0.30) as i32,
        sidebar + 2,
    )
    .unwrap_or((0, 0.0));

    // 标题栏缺失时无法可靠区分标题与消息区，判为低置信度。
    if title_bottom <= 0 || title_bottom >= separator_y {
        return Err(LayoutError::LowConfidence);
    }

    let normalize = |v: f32| (v / 48.0).clamp(0.0, 1.0);
    let confidence =
        (normalize(sidebar_strength) + normalize(sep_strength) + normalize(title_strength)) / 3.0;
    if confidence < 0.35 {
        return Err(LayoutError::LowConfidence);
    }

    Ok(LayoutResult {
        chat_title: RECT {
            left: sidebar,
            top: 0,
            right: width,
            bottom: title_bottom,
        },
        message_context: RECT {
            left: sidebar,
            top: title_bottom,
            right: width,
            bottom: separator_y,
        },
        compose_input: RECT {
            left: sidebar,
            top: separator_y,
            right: width,
            bottom: height,
        },
        sidebar_width: sidebar,
        separator_y,
        title_bottom,
        layout_signature: compute_signature(width, height, sidebar, title_bottom, separator_y),
        confidence,
    })
}

/// 两个矩形的 IoU，用于定位精度评估。
pub fn iou(a: RECT, b: RECT) -> f32 {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right.min(b.right);
    let bottom = a.bottom.min(b.bottom);
    if right <= left || bottom <= top {
        return 0.0;
    }
    let inter = ((right - left) as i64) * ((bottom - top) as i64);
    let area_a = ((a.right - a.left) as i64) * ((a.bottom - a.top) as i64);
    let area_b = ((b.right - b.left) as i64) * ((b.bottom - b.top) as i64);
    let union = area_a + area_b - inter;
    if union <= 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一张合成布局图，验证定位不依赖真实截图。
    fn synthetic(width: i32, height: i32, sidebar: i32, title_h: i32, compose_h: i32) -> Vec<u8> {
        let stride = (width * 4) as usize;
        let mut pixels = vec![0u8; stride * height as usize];
        let set = |p: &mut [u8], x: i32, y: i32, v: u8| {
            let idx = y as usize * stride + x as usize * 4;
            p[idx] = v;
            p[idx + 1] = v;
            p[idx + 2] = v;
            p[idx + 3] = 255;
        };
        for y in 0..height {
            for x in 0..width {
                let v = if x < sidebar {
                    220
                } else if y < title_h {
                    255
                } else if y < height - compose_h {
                    245
                } else {
                    255
                };
                set(&mut pixels, x, y, v);
            }
        }
        // 三条分隔线。
        for y in 0..height {
            set(&mut pixels, sidebar, y, 150);
        }
        for x in sidebar..width {
            set(&mut pixels, x, title_h, 150);
            set(&mut pixels, x, height - compose_h, 150);
        }
        pixels
    }

    #[test]
    fn detects_structure_on_synthetic_layout() {
        let (w, h, side, title, compose) = (1280, 800, 320, 72, 190);
        let pixels = synthetic(w, h, side, title, compose);
        let result = detect(&pixels, w, h, (w * 4) as usize).expect("应能定位");

        // 分隔线本身占一个像素，检测到的是线两侧之一，容差 2 像素。
        // 真实验收口径是 ROI 的 IoU，而不是边界的像素级相等。
        assert!(
            (result.sidebar_width - side).abs() <= 2,
            "侧栏边界 {} 偏离真值 {side} 超过容差",
            result.sidebar_width
        );
        assert!(
            (result.title_bottom - title).abs() <= 2,
            "标题下沿 {} 偏离真值 {title} 超过容差",
            result.title_bottom
        );
        assert!(
            (result.separator_y - (h - compose)).abs() <= 2,
            "输入区分隔线 {} 偏离真值 {} 超过容差",
            result.separator_y,
            h - compose
        );
        assert!(result.confidence > 0.35);
    }

    #[test]
    fn signature_is_stable_across_identical_proportions() {
        let a = detect(&synthetic(1280, 800, 320, 72, 190), 1280, 800, 1280 * 4).unwrap();
        let b = detect(&synthetic(1280, 800, 320, 72, 190), 1280, 800, 1280 * 4).unwrap();
        assert_eq!(a.layout_signature, b.layout_signature);
    }

    #[test]
    fn signature_differs_for_different_layout_proportions() {
        let a = detect(&synthetic(1280, 800, 320, 72, 190), 1280, 800, 1280 * 4).unwrap();
        let b = detect(&synthetic(1280, 800, 500, 72, 190), 1280, 800, 1280 * 4).unwrap();
        assert_ne!(a.layout_signature, b.layout_signature);
    }

    #[test]
    fn rejects_frames_below_minimum_size() {
        let pixels = synthetic(320, 240, 80, 20, 60);
        assert!(matches!(
            detect(&pixels, 320, 240, 320 * 4),
            Err(LayoutError::TooSmall)
        ));
    }

    #[test]
    fn rejects_uniform_frame_without_structure() {
        let w = 1280;
        let h = 800;
        let pixels = vec![200u8; (w * 4 * h) as usize];
        assert!(detect(&pixels, w, h, (w * 4) as usize).is_err());
    }

    #[test]
    fn iou_is_one_for_identical_rects() {
        let r = RECT {
            left: 10,
            top: 20,
            right: 110,
            bottom: 120,
        };
        assert!((iou(r, r) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn iou_is_zero_for_disjoint_rects() {
        let a = RECT {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        };
        let b = RECT {
            left: 20,
            top: 20,
            right: 30,
            bottom: 30,
        };
        assert_eq!(iou(a, b), 0.0);
    }
}
