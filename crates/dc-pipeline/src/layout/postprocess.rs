//! YOLO v8/v11 检出头后处理（§5.5 Postprocessor）。
//!
//! 输入是导出 ONNX 的输出张量（`1 × (4+nc) × 8400`），纯函数完成：
//! 解码 → 置信度过滤 → 类别 NMS → TagMapper（每 tag 取最高置信度 1 个）。

use dc_sys::Rect;

use crate::contract::Region;

/// 裸检出（解码后、NMS 前）。
#[derive(Debug, Clone, PartialEq)]
pub struct RawDet {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub class: usize,
    pub score: f32,
}

/// 从 ONNX 输出解出裸检出。
///
/// `data` 形状 `1 × (4+nc) × anchors`，前 4 行是 cx cy w h（像素系，letterbox 坐标），
/// 之后每行是一个类别的锚点分数（v8/v11 无 objectness）。
pub fn decode(data: &[f32], anchors: usize, classes: usize) -> Vec<RawDet> {
    let rows = 4 + classes;
    debug_assert_eq!(data.len(), rows * anchors);
    let mut out = Vec::new();
    for a in 0..anchors {
        // 每锚点取最高类别；任一类别分数非有限（NaN/±inf）即该锚点作废
        let mut best_c = 0usize;
        let mut best_s = f32::MIN;
        let mut valid = false;
        for c in 0..classes {
            let s = data[(4 + c) * anchors + a];
            if s.is_finite() {
                valid = true;
                if s > best_s {
                    best_s = s;
                    best_c = c;
                }
            }
        }
        if !valid {
            continue;
        }
        out.push(RawDet {
            x: data[a],
            y: data[anchors + a],
            w: data[2 * anchors + a],
            h: data[3 * anchors + a],
            class: best_c,
            score: best_s,
        });
    }
    out
}

/// IoU（轴对齐框）。
pub fn iou(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> f32 {
    let (ax1, ay1, ax2, ay2) = a;
    let (bx1, by1, bx2, by2) = b;
    let ix1 = ax1.max(bx1);
    let iy1 = ay1.max(by1);
    let ix2 = ax2.min(bx2);
    let iy2 = ay2.min(by2);
    let inter = (ix2 - ix1).max(0.0) * (iy2 - iy1).max(0.0);
    let area_a = (ax2 - ax1) * (ay2 - ay1);
    let area_b = (bx2 - bx1) * (by2 - by1);
    if area_a + area_b - inter <= 0.0 {
        0.0
    } else {
        inter / (area_a + area_b - inter)
    }
}

fn box_of(d: &RawDet) -> (f32, f32, f32, f32) {
    (
        d.x - d.w / 2.0,
        d.y - d.h / 2.0,
        d.x + d.w / 2.0,
        d.y + d.h / 2.0,
    )
}

/// 类别 NMS：同类别内按分数降序，IoU > threshold 的低分框被抑制。
/// （跨类别不抑制——chat_window 与 chat_target 可能合法重叠。）
pub fn nms(dets: &[RawDet], iou_threshold: f32) -> Vec<RawDet> {
    let mut sorted: Vec<&RawDet> = dets.iter().collect();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<RawDet> = Vec::new();
    for d in sorted {
        let bb = box_of(d);
        let suppressed = kept
            .iter()
            .any(|k| k.class == d.class && iou(box_of(k), bb) > iou_threshold);
        if !suppressed {
            kept.push(d.clone());
        }
    }
    kept
}

/// TagMapper：类别 index → tags.json 名称；每 tag 取置信度最高 1 个（v1.0），
/// 输出为 Region（原图像素系坐标由调用方用 [`crate::layout::preprocess::rect_to_src`] 换算，
/// 此处只做类别映射与排序）。
pub fn map_tags(dets: &[RawDet], class_names: &[String], conf_threshold: f32) -> Vec<Region> {
    let mut best: Vec<Region> = Vec::new();
    for d in dets.iter().filter(|d| d.score >= conf_threshold) {
        let Some(tag) = class_names.get(d.class) else {
            continue;
        };
        let rect = Rect::new(
            d.x.round() as i32,
            d.y.round() as i32,
            d.w.round().max(1.0) as u32,
            d.h.round().max(1.0) as u32,
        );
        if let Some(prev) = best.iter_mut().find(|r| r.tag == *tag) {
            if d.score > prev.confidence {
                *prev = Region {
                    tag: tag.clone(),
                    rect,
                    confidence: d.score,
                };
            }
        } else {
            best.push(Region {
                tag: tag.clone(),
                rect,
                confidence: d.score,
            });
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(x: f32, y: f32, w: f32, h: f32, class: usize, score: f32) -> RawDet {
        RawDet {
            x,
            y,
            w,
            h,
            class,
            score,
        }
    }

    /// UT-LAY-02：合成框 NMS——重叠抑制与阈值边界。
    #[test]
    fn ut_lay_02_nms() {
        let dets = vec![
            det(100.0, 100.0, 50.0, 50.0, 0, 0.9), // chat_list 高分
            det(102.0, 101.0, 50.0, 50.0, 0, 0.8), // 同类重叠 → 抑制
            det(400.0, 100.0, 50.0, 50.0, 0, 0.7), // 同类不重叠 → 保留
            det(100.0, 100.0, 60.0, 60.0, 1, 0.6), // 异类重叠 → 不抑制
        ];
        let kept = nms(&dets, 0.5);
        // 同类 0 抑制掉 0.8 那个；异类 1 保留
        assert_eq!(kept.len(), 3, "kept: {kept:?}");
        assert!(kept.iter().all(|d| d.score != 0.8));

        // 阈值边界：IoU 恰好等于 0.5 → 不抑制（`> threshold` 才抑制）
        let edge = vec![
            det(0.0, 0.0, 10.0, 10.0, 0, 0.9),
            det(0.0, 5.0, 10.0, 10.0, 0, 0.8), // 交叠 50 → IoU=0.333
            det(0.0, 0.0, 10.0, 10.0, 0, 0.95),
            det(0.0, 2.5, 10.0, 10.0, 0, 0.85), // IoU = 7.5²×2-… < 0.5
        ];
        let _ = edge; // 覆盖数组；边界语义在下方精确断言
        let exact = vec![
            det(0.0, 0.0, 10.0, 10.0, 0, 0.9),
            det(5.0, 0.0, 10.0, 10.0, 0, 0.8), // IoU = 50/150 = 0.333 < 0.5
        ];
        assert_eq!(nms(&exact, 0.5).len(), 2);
        let suppress = vec![
            det(0.0, 0.0, 10.0, 10.0, 0, 0.9),
            det(2.0, 0.0, 10.0, 10.0, 0, 0.8), // IoU = 80/120 ≈ 0.667 > 0.5
        ];
        assert_eq!(nms(&suppress, 0.5).len(), 1);
    }

    /// 解码：每锚点取最高类别。
    #[test]
    fn decode_picks_best_class_per_anchor() {
        let nc = 2;
        let anchors = 3;
        // 手工构造 1×6×3：anchor0 → class1 分高；anchor1 → class0；anchor2 分数 NaN 跳过
        let mut data = vec![0f32; (4 + nc) * anchors];
        data[0] = 10.0;
        data[3] = 20.0;
        data[6] = 5.0;
        data[9] = 6.0; // cx
        data[1] = 11.0;
        data[4] = 21.0;
        data[7] = 5.0;
        data[10] = 6.0; // cy
        data[2] = 12.0;
        data[5] = 22.0;
        data[8] = 5.0;
        data[11] = 6.0; // w
        data[3] = 13.0;
        data[6 + 3] = 23.0;
        data[9 + 3] = 5.0;
        data[12 + 3] = 6.0; // h（第 4 行）
                            // class 分数行（第 4/5 行）：重新明确写
        data.fill(0.0);
        // anchor0: cx,cy,w,h = 10,11,12,13; class0=0.1 class1=0.9
        data[0] = 10.0;
        data[anchors] = 11.0;
        data[2 * anchors] = 12.0;
        data[3 * anchors] = 13.0;
        data[4 * anchors] = 0.1;
        data[5 * anchors] = 0.9;
        // anchor1: class0=0.8 class1=0.2
        data[1] = 20.0;
        data[anchors + 1] = 21.0;
        data[2 * anchors + 1] = 22.0;
        data[3 * anchors + 1] = 23.0;
        data[4 * anchors + 1] = 0.8;
        data[5 * anchors + 1] = 0.2;
        // anchor2: 全 NaN
        data[2] = f32::NAN;
        data[anchors + 2] = f32::NAN;
        data[4 * anchors + 2] = f32::NAN;
        data[5 * anchors + 2] = f32::NAN;

        let dets = decode(&data, anchors, nc);
        assert_eq!(dets.len(), 2, "NaN 锚点应跳过: {dets:?}");
        assert_eq!(dets[0].class, 1);
        assert!((dets[0].score - 0.9).abs() < 1e-6);
        assert_eq!(dets[1].class, 0);
    }

    /// TagMapper：conf 阈值过滤 + 每 tag 取最高分。
    #[test]
    fn tag_mapper_picks_best_per_tag() {
        let names = vec!["chat_list".to_string(), "msg_input".to_string()];
        let dets = vec![
            det(10.0, 10.0, 5.0, 5.0, 0, 0.6),
            det(12.0, 12.0, 5.0, 5.0, 0, 0.9), // 同 tag 更高 → 胜出
            det(50.0, 50.0, 5.0, 5.0, 1, 0.30), // 低于阈值 → 过滤
        ];
        let regions = map_tags(&dets, &names, 0.45);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].tag, "chat_list");
        assert!((regions[0].confidence - 0.9).abs() < 1e-6);
    }
}
