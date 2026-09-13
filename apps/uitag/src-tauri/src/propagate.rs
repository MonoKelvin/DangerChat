use std::path::Path;

use image::{DynamicImage, GenericImageView, ImageBuffer, Luma};
use imageproc::template_matching::{match_template, MatchTemplateMethod};
use serde::{Deserialize, Serialize};

use crate::state::AnnoBox;

/// 传播请求：以 src_path 的标注为模板，应用到 targets。
#[derive(Debug, Clone, Deserialize)]
pub struct PropagateRequest {
    pub src_path: String,
    pub boxes: Vec<AnnoBox>,
    pub targets: Vec<String>,
    /// NCC 峰值低于此值的框不落标注（宁缺勿错）
    pub min_confidence: f64,
    /// 允许目标图与模板尺寸不同：先把模板缩放到目标尺度再匹配
    pub allow_rescale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PropagatedBox {
    #[serde(flatten)]
    pub box_: AnnoBox,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PropagateResult {
    pub path: String,
    pub boxes: Vec<PropagatedBox>,
}

/// 匹配工作尺度：模板长边压到此值以内。
/// 大区域（chat_window ~700px）的 NCC 在全分辨率是万亿次运算，压到 160px 定位
/// 误差 ≤ 3%（×回原尺度约 ±20px），对「预标注待人修」的精度完全够。
const MAX_TEMPL_SIDE: u32 = 160;
/// 粗定位降采样比（相对工作尺度再降 4×）
const COARSE_FACTOR: u32 = 4;

/// 单框匹配：把 box 区域当模板，在 target 上找 NCC 峰值。
/// 三步（性能关键）：
///   1. 模板缩放到工作尺度（长边 ≤ 160px）
///   2. 工作尺度的 1/4 全图粗定位
///   3. 工作尺度 ±16px 邻域精化
///
/// 返回 None 表示模板比目标大（无法匹配）或解码失败。
fn match_one(
    tmpl_img: &DynamicImage,
    box_: &AnnoBox,
    target: &DynamicImage,
    allow_rescale: bool,
) -> Option<(AnnoBox, f64)> {
    let (tw, th) = tmpl_img.dimensions();
    let (gw, gh) = target.dimensions();

    let bx = box_.x.round().clamp(0.0, (tw as f64) - 1.0) as u32;
    let by = box_.y.round().clamp(0.0, (th as f64) - 1.0) as u32;
    let bw = box_.w.round().max(1.0) as u32;
    let bh = box_.h.round().max(1.0) as u32;
    if bx + bw > tw || by + bh > th {
        return None;
    }

    // 目标与模板图尺寸不同：按高度比缩放（微信窗口截图宽度受窗口拉伸影响，高度更稳定）
    let scale = if (tw != gw || th != gh) && allow_rescale {
        gh as f64 / th as f64
    } else if tw != gw || th != gh {
        return None;
    } else {
        1.0
    };

    let tmpl_rgb = tmpl_img.crop_imm(bx, by, bw, bh).to_luma8();
    let scaled = if (scale - 1.0).abs() > 0.02 {
        let (nw, nh) = ((bw as f64 * scale) as u32, (bh as f64 * scale) as u32);
        if nw == 0 || nh == 0 || nw > gw || nh > gh {
            return None;
        }
        DynamicImage::ImageLuma8(tmpl_rgb)
            .resize_exact(nw, nh, image::imageops::FilterType::Triangle)
            .to_luma8()
    } else {
        tmpl_rgb
    };

    // ── 工作尺度：整图与模板等比缩放，记录比例 ──
    let (mw0, mh0) = scaled.dimensions();
    let work = MAX_TEMPL_SIDE.max(32) as f64 / mw0.max(mh0).max(1) as f64;
    let work = work.min(1.0); // 只缩不放
    let (wm, hm) = ((mw0 as f64 * work) as u32, (mh0 as f64 * work) as u32);
    if wm < 8 || hm < 8 {
        return None;
    }
    let tmpl_work = DynamicImage::ImageLuma8(scaled)
        .resize_exact(wm, hm, image::imageops::FilterType::Triangle)
        .to_luma8();
    let (gw_w, gh_w) = ((gw as f64 * work) as u32, (gh as f64 * work) as u32);
    let target_work = DynamicImage::ImageLuma8(target.to_luma8())
        .resize_exact(gw_w, gh_w, image::imageops::FilterType::Triangle)
        .to_luma8();

    // ── 粗定位：工作尺度的 1/4 ──
    let c = COARSE_FACTOR;
    let (tw_c, th_c) = ((gw_w / c).max(wm / c + 1), (gh_w / c).max(hm / c + 1));
    let target_coarse = DynamicImage::ImageLuma8(target_work.clone())
        .resize_exact(tw_c, th_c, image::imageops::FilterType::Triangle)
        .to_luma8();
    let tmpl_coarse = DynamicImage::ImageLuma8(tmpl_work.clone())
        .resize_exact((wm / c).max(2), (hm / c).max(2), image::imageops::FilterType::Triangle)
        .to_luma8();
    if tmpl_coarse.width() > target_coarse.width() || tmpl_coarse.height() > target_coarse.height() {
        return None;
    }
    let coarse_map = match_template(
        &target_coarse,
        &tmpl_coarse,
        MatchTemplateMethod::SumOfSquaredErrorsNormalized,
    );
    let (cx, cy, _) = find_min_sse(&coarse_map);

    // ── 精化：工作尺度 ±16px 邻域（粗定位误差 ≤ 1 粗像素 × c = 4 工作像素）──
    let margin = 16u32;
    let sx = (cx * c).saturating_sub(margin);
    let sy = (cy * c).saturating_sub(margin);
    let ex = ((cx * c) + wm + margin).min(gw_w);
    let ey = ((cy * c) + hm + margin).min(gh_w);
    let (rw, rh) = (ex - sx, ey - sy);
    if rw < wm || rh < hm {
        return None;
    }
    let region = target_work.view(sx, sy, rw, rh).to_image();
    let fine_map = match_template(
        &region,
        &tmpl_work,
        MatchTemplateMethod::SumOfSquaredErrorsNormalized,
    );
    let (fx, fy, ncc) = find_min_sse(&fine_map);

    // 工作尺度坐标 → 原尺度
    let inv = 1.0 / work;
    let found = AnnoBox {
        tag: box_.tag.clone(),
        x: ((sx + fx) as f64) * inv,
        y: ((sy + fy) as f64) * inv,
        w: wm as f64 * inv,
        h: hm as f64 * inv,
    };
    Some((found, ncc))
}

/// SSE 归一化分数图上找最小值（误差最小 = 最佳匹配）。
/// 完全匹配时值 = 0，转换成「相似度」用 1 - sse（近似 NCC 语义，完全匹配 = 1）。
fn find_min_sse(img: &ImageBuffer<Luma<f32>, Vec<f32>>) -> (u32, u32, f64) {
    let mut best = (0u32, 0u32, f32::MAX);
    for (x, y, p) in img.enumerate_pixels() {
        if p[0] < best.2 {
            best = (x, y, p[0]);
        }
    }
    (best.0, best.1, (1.0 - best.2 as f64).clamp(0.0, 1.0))
}

pub fn propagate(req: &PropagateRequest) -> Result<Vec<PropagateResult>, String> {
    let tmpl_img = image::open(Path::new(&req.src_path))
        .map_err(|e| format!("模板图读取失败 {}: {e}", req.src_path))?;

    let mut out = Vec::with_capacity(req.targets.len());
    for t in &req.targets {
        let target = match image::open(Path::new(t)) {
            Ok(img) => img,
            Err(e) => {
                return Err(format!("目标图读取失败 {t}: {e}"));
            }
        };
        let mut boxes = Vec::new();
        for b in &req.boxes {
            if let Some((found, conf)) = match_one(&tmpl_img, b, &target, req.allow_rescale) {
                if conf >= req.min_confidence {
                    boxes.push(PropagatedBox { box_: found, confidence: conf });
                }
            }
        }
        out.push(PropagateResult { path: t.clone(), boxes });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    /// 合成图：白底 + 一个固定位置的深灰矩形（模拟输入框）。
    fn synth(path: &std::path::Path, rect: (u32, u32, u32, u32), w: u32, h: u32) {
        let mut img = ImageBuffer::from_pixel(w, h, Rgb([240u8, 240, 240]));
        for y in rect.1..rect.1 + rect.3 {
            for x in rect.0..rect.0 + rect.2 {
                img.put_pixel(x, y, Rgb([70, 70, 70]));
            }
        }
        // 加点噪点让 NCC 不完美但峰值明确
        for i in 0..(w * h / 97).min(2000) {
            let x = (i * 37) % w;
            let y = (i * 53) % h;
            img.put_pixel(x, y, Rgb([200, 200, 200]));
        }
        img.save(path).unwrap();
    }

    #[test]
    fn propagates_exact_same_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        synth(&a, (100, 200, 300, 80), 800, 600);
        synth(&b, (100, 200, 300, 80), 800, 600);

        let req = PropagateRequest {
            src_path: a.to_string_lossy().into_owned(),
            boxes: vec![AnnoBox { tag: "msg_input".into(), x: 100.0, y: 200.0, w: 300.0, h: 80.0 }],
            targets: vec![b.to_string_lossy().into_owned()],
            min_confidence: 0.6,
            allow_rescale: true,
        };
        let results = propagate(&req).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].boxes.len(), 1);
        let found = &results[0].boxes[0];
        assert_eq!(found.box_.tag, "msg_input");
        assert!((found.box_.x - 100.0).abs() <= 2.0, "x={}", found.box_.x);
        assert!((found.box_.y - 200.0).abs() <= 2.0, "y={}", found.box_.y);
        assert!(found.confidence > 0.9, "conf={}", found.confidence);
    }

    #[test]
    fn propagates_shifted_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        // 同布局但元素平移了 (40, 30)
        synth(&a, (100, 200, 300, 80), 800, 600);
        synth(&b, (140, 230, 300, 80), 800, 600);

        let req = PropagateRequest {
            src_path: a.to_string_lossy().into_owned(),
            boxes: vec![AnnoBox { tag: "msg_input".into(), x: 100.0, y: 200.0, w: 300.0, h: 80.0 }],
            targets: vec![b.to_string_lossy().into_owned()],
            min_confidence: 0.6,
            allow_rescale: true,
        };
        let results = propagate(&req).unwrap();
        let found = &results[0].boxes[0];
        assert!((found.box_.x - 140.0).abs() <= 3.0, "x={}", found.box_.x);
        assert!((found.box_.y - 230.0).abs() <= 3.0, "y={}", found.box_.y);
    }

    #[test]
    fn skips_when_confidence_low() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        // b 是纯噪声，与模板无对应关系
        let mut img = ImageBuffer::new(800, 600);
        for y in 0..600 {
            for x in 0..800 {
                img.put_pixel(x, y, Rgb([(x * 31 % 255) as u8, (y * 17 % 255) as u8, 90]));
            }
        }
        DynamicImage::ImageRgb8(img).save(&b).unwrap();
        synth(&a, (100, 200, 300, 80), 800, 600);

        let req = PropagateRequest {
            src_path: a.to_string_lossy().into_owned(),
            boxes: vec![AnnoBox { tag: "msg_input".into(), x: 100.0, y: 200.0, w: 300.0, h: 80.0 }],
            targets: vec![b.to_string_lossy().into_owned()],
            min_confidence: 0.99, // 抬高阈值：噪声图不可能完全匹配
            allow_rescale: true,
        };
        let results = propagate(&req).unwrap();
        assert_eq!(results[0].boxes.len(), 0, "低置信度应跳过");
    }

    #[test]
    fn rescales_to_different_size() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        // 目标图整体 1.5×：元素位置与尺寸都放大
        synth(&a, (100, 200, 300, 80), 800, 600);
        synth(&b, (150, 300, 450, 120), 1200, 900);

        let req = PropagateRequest {
            src_path: a.to_string_lossy().into_owned(),
            boxes: vec![AnnoBox { tag: "msg_input".into(), x: 100.0, y: 200.0, w: 300.0, h: 80.0 }],
            targets: vec![b.to_string_lossy().into_owned()],
            min_confidence: 0.6,
            allow_rescale: true,
        };
        let results = propagate(&req).unwrap();
        let found = &results[0].boxes[0];
        assert!((found.box_.x - 150.0).abs() <= 6.0, "x={}", found.box_.x);
        assert!((found.box_.w - 450.0).abs() <= 10.0, "w={}", found.box_.w);
    }
}
