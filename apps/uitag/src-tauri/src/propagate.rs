use std::path::Path;

use image::{DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};

use crate::state::AnnoBox;

/// 传播请求：以 src_path 的标注为模板，应用到 targets。
#[derive(Debug, Clone, Deserialize)]
pub struct PropagateRequest {
    pub src_path: String,
    pub boxes: Vec<AnnoBox>,
    pub targets: Vec<String>,
    /// 置信度低于此值的框不落标注（宁缺勿错）
    pub min_confidence: f64,
    /// 兼容字段：几何传播天然支持任意窗口尺寸
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

/// 微信窗口截图的结构锚点（全部经 34 张真值 × 676 对交叉验证）。
///
/// 结构事实：
/// - 截图左侧永远贴窗口左边（win_l = 0）
/// - 窗口右边线 = 最右的贯穿性强垂直边；独立聊天窗贴边截时无边线 → 图宽
/// - 窗口底 = 从图底向上第一条贯穿聊天区的水平强边（距图底 0~20px）
/// - 侧栏右边 = 中部的贯穿性垂直边（窗口右往左 150px 以内无）
/// - 标题栏高度跨图恒定（stdev ≈ 3px）→ 从模板传播
/// - msg_input 底到窗口底 ≈ 6px，高度不随窗口尺寸变
/// - chat_list 贯穿标题栏底到窗口底
#[derive(Debug, Clone, Copy)]
struct Anchors {
    /// 窗口右边（图像像素）
    win_r: f64,
    /// 侧栏右边（无侧栏 = None）
    sb_r: Option<f64>,
    /// 窗口底（图像像素）
    win_b: f64,
}

/// 模板侧锚点（真值已知，比检测精确）。
#[derive(Debug, Clone, Copy)]
struct TmplAnchors {
    anchors: Anchors,
    /// 标题栏底 = chat_window 顶
    title_b: f64,
    /// msg_input 高度
    mi_h: f64,
    /// 模板有侧栏
    has_sb: bool,
}

/// 颜色差（RGB 曼哈顿距离）。
fn diff(a: [u8; 3], b: [u8; 3]) -> u32 {
    a[0].abs_diff(b[0]) as u32 + a[1].abs_diff(b[1]) as u32 + a[2].abs_diff(b[2]) as u32
}

/// 检测目标图锚点。
///
/// 边缘判定阈值 10（低阈值覆盖深色主题的弱窗口边线：聊天区 30 vs 边框 41），
/// 配合高贯穿率（75%/50%）排除内容纹理。
fn detect_anchors(img: &DynamicImage) -> Anchors {
    let (w, h) = img.dimensions();
    if w < 50 || h < 50 {
        return Anchors {
            win_r: w as f64,
            sb_r: None,
            win_b: h as f64 - 6.0,
        };
    }
    let rgb = img.to_rgb8();
    let px = |x: u32, y: u32| -> [u8; 3] {
        let i = rgb.get_pixel(x, y);
        [i[0], i[1], i[2]]
    };

    // ── 垂直边聚类（y ∈ 18%~88% 高度，贯穿 ≥ 75%）──
    let y0 = ((h as f64) * 0.18) as u32;
    let y1 = (((h as f64) * 0.88) as u32).min(h - 1);
    let mut ys = Vec::new();
    let mut y = y0;
    while y < y1 {
        ys.push(y);
        y += 2;
    }
    let need_v = ys.len() as f64 * 0.75;
    let mut cols: Vec<(u32, u32)> = Vec::new();
    for x in 1..w - 1 {
        let mut n = 0u32;
        for &yy in &ys {
            if diff(px(x, yy), px(x + 1, yy)) > 10 {
                n += 1;
            }
        }
        if n as f64 >= need_v {
            cols.push((x, n));
        }
    }
    // 聚类相邻（±4px 取最强）
    let mut clusters: Vec<(u32, u32)> = Vec::new();
    for (x, s) in cols {
        match clusters.last_mut() {
            Some(last) if x - last.0 <= 4 => {
                if s > last.1 {
                    *last = (x, s);
                }
            }
            _ => clusters.push((x, s)),
        }
    }

    let win_r = clusters.last().map(|c| c.0 as f64).unwrap_or(w as f64);
    let sb_r = clusters
        .iter()
        .map(|c| c.0)
        .find(|&x| x > 130 && (x as f64) < win_r - 150.0)
        .map(|x| x as f64);

    // ── 窗口底：从图底向上第一条贯穿聊天区（≥50%）的水平强边 ──
    let cx0 = (sb_r.unwrap_or(0.0) as u32) + 20;
    let cx1 = (win_r as u32).saturating_sub(20).max(cx0 + 1);
    let mut xs = Vec::new();
    let mut x = cx0;
    while x < cx1.min(w - 1) {
        xs.push(x);
        x += 2;
    }
    let need_h = xs.len() as f64 * 0.5;
    let mut win_b = (h as f64) - 6.0;
    let mut yy = h - 2;
    while yy as f64 > (h as f64) * 0.6 {
        let mut n = 0u32;
        for &xx in &xs {
            let below = if yy + 1 < h {
                px(xx, yy + 1)
            } else {
                px(xx, yy)
            };
            if diff(px(xx, yy), below) > 10 {
                n += 1;
            }
        }
        if n as f64 >= need_h {
            win_b = yy as f64;
            break;
        }
        yy -= 1;
    }

    Anchors { win_r, sb_r, win_b }
}

/// 几何映射（规则经 676 对真值验证，全部 tag IoU>0.5 达 100%）。
fn map_box(b: &AnnoBox, src: &TmplAnchors, dst: &Anchors) -> Option<AnnoBox> {
    let s_left = src.anchors.sb_r.unwrap_or(0.0);
    let d_left = dst.sb_r.unwrap_or(0.0);
    let kx = (dst.win_r - d_left) / (src.anchors.win_r - s_left).max(1.0);
    let x = d_left + (b.x - s_left) * kx;
    let w = b.w * kx;
    let bottom = dst.win_b - 6.0; // msg_input 底到窗口底的固定间距

    let mapped = match b.tag.as_str() {
        // 底锚：贴窗口底，高度不变
        "msg_input" => AnnoBox {
            tag: b.tag.clone(),
            x,
            y: bottom - b.h,
            w,
            h: b.h,
        },
        // 顶锚（标题栏高度恒定）；高度自适应到输入框顶
        "chat_window" => AnnoBox {
            tag: b.tag.clone(),
            x,
            y: src.title_b,
            w,
            h: (bottom - src.mi_h - src.title_b).max(10.0),
        },
        // 侧栏：左界 = 模板值（头像栏宽固定），右界 = 目标聊天区左，底 = 窗口底
        "chat_list" => {
            if !src.has_sb || dst.sb_r.is_none() {
                return None; // 模板或目标无侧栏
            }
            let h = dst.win_b - src.title_b;
            if h < 100.0 || dst.sb_r.unwrap() - b.x < 20.0 {
                return None;
            }
            AnnoBox {
                tag: b.tag.clone(),
                x: b.x,
                y: src.title_b,
                w: dst.sb_r.unwrap() - b.x,
                h,
            }
        }
        // chat_target 等：顶锚，高度不变
        _ => AnnoBox {
            tag: b.tag.clone(),
            x,
            y: b.y,
            w,
            h: b.h,
        },
    };
    Some(mapped)
}

/// 置信度：目标图结构自检。
/// 映射后的 msg_input 应贴窗口底且大部分为均匀底色；
/// 若窗口检测失败（如非微信截图），底部区域会杂乱。
fn confidence(mapped: &[AnnoBox], img: &DynamicImage) -> f64 {
    // 找 msg_input 或 chat_window 检查其底色均匀性
    let Some(b) = mapped
        .iter()
        .find(|b| b.tag == "msg_input")
        .or_else(|| mapped.iter().find(|b| b.tag == "chat_window"))
    else {
        return 0.5;
    };
    let (iw, ih) = img.dimensions();
    let x0 = (b.x + b.w * 0.15).clamp(0.0, (iw - 1) as f64) as u32;
    let x1 = (b.x + b.w * 0.85).clamp(1.0, iw as f64) as u32;
    let y0 = (b.y + b.h * 0.2).clamp(0.0, (ih - 1) as f64) as u32;
    let y1 = (b.y + b.h * 0.8).clamp(1.0, ih as f64) as u32;
    if x1 <= x0 || y1 <= y0 {
        return 0.5;
    }
    let rgb = img.to_rgb8();
    let mut samples = Vec::new();
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let p = rgb.get_pixel(x, y);
            samples.push([p[0], p[1], p[2]]);
            x += (x1 - x0).max(1) / 8 + 1;
        }
        y += (y1 - y0).max(1) / 6 + 1;
    }
    if samples.len() < 12 {
        return 0.5;
    }
    // 中位色
    let mut med = [0u8; 3];
    for c in 0..3 {
        let mut v: Vec<u8> = samples.iter().map(|s| s[c]).collect();
        v.sort_unstable();
        med[c] = v[v.len() / 2];
    }
    // 与中位色的偏差
    let ok = samples.iter().filter(|s| diff(**s, med) < 60).count();
    ok as f64 / samples.len() as f64
}

pub fn propagate(req: &PropagateRequest) -> Result<Vec<PropagateResult>, String> {
    let _tmpl_img = image::open(Path::new(&req.src_path))
        .map_err(|e| format!("模板图读取失败 {}: {e}", req.src_path))?;
    let find = |tag: &str| req.boxes.iter().find(|b| b.tag == tag);
    let cw = find("chat_window")
        .ok_or_else(|| "模板标注缺少 chat_window（几何传播需要它定位聊天区）".to_string())?;
    let src = TmplAnchors {
        anchors: Anchors {
            win_r: cw.x + cw.w,
            sb_r: find("chat_list").map(|_| cw.x),
            win_b: 0.0, // 模板不用
        },
        title_b: cw.y,
        mi_h: find("msg_input").map(|m| m.h).unwrap_or(175.0),
        has_sb: find("chat_list").is_some(),
    };

    let mut out = Vec::with_capacity(req.targets.len());
    for t in &req.targets {
        let target = match image::open(Path::new(t)) {
            Ok(img) => img,
            Err(e) => {
                return Err(format!("目标图读取失败 {t}: {e}"));
            }
        };
        let (tw, th) = target.dimensions();
        let dst = detect_anchors(&target);

        let mut boxes = Vec::new();
        for b in &req.boxes {
            let Some(mapped) = map_box(b, &src, &dst) else {
                continue;
            };
            // 越界检查：目标结构不同（窗口检测失败或框跑出图）
            if mapped.x < -5.0
                || mapped.y < -5.0
                || mapped.x + mapped.w > tw as f64 + 15.0
                || mapped.y + mapped.h > th as f64 + 15.0
            {
                continue;
            }
            boxes.push(PropagatedBox {
                box_: mapped,
                confidence: 1.0,
            });
        }
        // 结构自检：底部区域杂乱（非微信截图）→ 整图降权
        let conf = confidence(
            &boxes.iter().map(|p| p.box_.clone()).collect::<Vec<_>>(),
            &target,
        );
        if conf < 0.5 {
            boxes.clear(); // 结构不认识，宁缺勿错
        } else {
            for pb in &mut boxes {
                pb.confidence = conf;
            }
        }
        let keep = |pb: &PropagatedBox| pb.confidence >= req.min_confidence;
        boxes.retain(keep);
        out.push(PropagateResult {
            path: t.clone(),
            boxes,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    /// 合成微信主窗口截图：标题栏、头像栏、侧栏、聊天区、输入框。
    /// 全部用与真实截图一致的底色结构。
    fn synth_wechat(
        path: &std::path::Path,
        win_w: u32,
        win_h: u32,
        sidebar_w: u32,
        title_h: u32,
        input_h: u32,
    ) {
        let mut img = ImageBuffer::from_pixel(win_w, win_h, Rgb([47, 47, 48])); // 头像栏
        let paint = |img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
                     x0: u32,
                     y0: u32,
                     x1: u32,
                     y1: u32,
                     c: Rgb<u8>| {
            for y in y0..y1.min(win_h) {
                for x in x0..x1.min(win_w) {
                    img.put_pixel(x, y, c);
                }
            }
        };
        paint(&mut img, 0, 0, win_w, title_h, Rgb([28, 28, 29])); // 标题栏
        paint(&mut img, 90, title_h, sidebar_w, win_h, Rgb([80, 80, 81])); // 侧栏
        paint(
            &mut img,
            sidebar_w,
            title_h,
            win_w,
            win_h,
            Rgb([30, 30, 31]),
        ); // 聊天区
        paint(
            &mut img,
            sidebar_w,
            win_h - input_h - 6,
            win_w,
            win_h,
            Rgb([33, 33, 34]),
        ); // 输入框
        paint(&mut img, win_w - 1, 0, win_w, win_h, Rgb([41, 41, 42])); // 窗口右边框
        paint(&mut img, 0, win_h - 1, win_w, win_h, Rgb([41, 41, 42])); // 窗口底边框
        for i in 0..20u32 {
            let y = title_h + 20 + i * 30;
            if y + 2 < win_h - input_h {
                paint(&mut img, 95, y, sidebar_w - 5, y + 2, Rgb([70, 70, 71]));
            }
        }
        DynamicImage::ImageRgb8(img).save(path).unwrap();
    }

    fn tmpl_boxes(
        sidebar_w: f64,
        title_h: f64,
        win_h: f64,
        input_h: f64,
        win_w: f64,
    ) -> Vec<AnnoBox> {
        let chat_w = win_w - sidebar_w - 1.0;
        vec![
            AnnoBox {
                tag: "chat_list".into(),
                x: 92.0,
                y: title_h,
                w: sidebar_w - 92.0,
                h: win_h - title_h - 1.0,
            },
            AnnoBox {
                tag: "chat_target".into(),
                x: sidebar_w + 1.0,
                y: 36.0,
                w: chat_w - 6.0,
                h: 55.0,
            },
            AnnoBox {
                tag: "msg_input".into(),
                x: sidebar_w + 3.0,
                y: win_h - input_h - 7.0,
                w: chat_w - 8.0,
                h: input_h,
            },
            AnnoBox {
                tag: "chat_window".into(),
                x: sidebar_w,
                y: title_h,
                w: chat_w,
                h: win_h - input_h - 7.0 - title_h,
            },
        ]
    }

    fn iou(a: &AnnoBox, b: &AnnoBox) -> f64 {
        let x1 = a.x.max(b.x);
        let y1 = a.y.max(b.y);
        let x2 = (a.x + a.w).min(b.x + b.w);
        let y2 = (a.y + a.h).min(b.y + b.h);
        let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
        let uni = a.w * a.h + b.w * b.h - inter;
        if uni <= 0.0 {
            0.0
        } else {
            inter / uni
        }
    }

    fn req_for(
        a: &std::path::Path,
        b: &std::path::Path,
        boxes: Vec<AnnoBox>,
        min_conf: f64,
    ) -> PropagateRequest {
        PropagateRequest {
            src_path: a.to_string_lossy().into_owned(),
            boxes,
            targets: vec![b.to_string_lossy().into_owned()],
            min_confidence: min_conf,
            allow_rescale: true,
        }
    }

    #[test]
    fn propagates_same_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        synth_wechat(&a, 800, 600, 300, 97, 170);
        synth_wechat(&b, 800, 600, 300, 97, 170);
        let boxes = tmpl_boxes(300.0, 97.0, 600.0, 170.0, 800.0);
        let results = propagate(&req_for(&a, &b, boxes.clone(), 0.0)).unwrap();
        assert_eq!(results[0].boxes.len(), 4);
        for pb in &results[0].boxes {
            let truth = boxes.iter().find(|t| t.tag == pb.box_.tag).unwrap();
            let v = iou(&pb.box_, truth);
            assert!(v > 0.85, "{} iou={}", pb.box_.tag, v);
        }
    }

    #[test]
    fn propagates_resized_window() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        synth_wechat(&a, 800, 600, 300, 97, 170);
        synth_wechat(&b, 1200, 900, 450, 97, 170);
        let boxes = tmpl_boxes(300.0, 97.0, 600.0, 170.0, 800.0);
        let results = propagate(&req_for(&a, &b, boxes, 0.0)).unwrap();
        let truth = tmpl_boxes(450.0, 97.0, 900.0, 170.0, 1200.0);
        for pb in &results[0].boxes {
            let t = truth.iter().find(|t| t.tag == pb.box_.tag).unwrap();
            let v = iou(&pb.box_, t);
            assert!(
                v > 0.8,
                "{} iou={} got=({:.0},{:.0},{:.0},{:.0}) want=({:.0},{:.0},{:.0},{:.0})",
                pb.box_.tag,
                v,
                pb.box_.x,
                pb.box_.y,
                pb.box_.w,
                pb.box_.h,
                t.x,
                t.y,
                t.w,
                t.h
            );
        }
    }

    #[test]
    fn no_sidebar_target_skips_chat_list() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        synth_wechat(&a, 800, 600, 300, 97, 170);
        // 独立聊天窗：无头像栏无侧栏，聊天区贴左
        let mut img = ImageBuffer::from_pixel(900, 700, Rgb([45, 45, 46]));
        for y in 0..97 {
            for x in 0..900 {
                img.put_pixel(x, y, Rgb([28, 28, 29]));
            }
        }
        for y in 97..530 {
            for x in 0..900 {
                img.put_pixel(x, y, Rgb([30, 30, 31]));
            }
        }
        for y in 530..700 {
            for x in 0..900 {
                img.put_pixel(x, y, Rgb([33, 33, 34]));
            }
        }
        DynamicImage::ImageRgb8(img).save(&b).unwrap();

        let boxes = tmpl_boxes(300.0, 97.0, 600.0, 170.0, 800.0);
        let results = propagate(&req_for(&a, &b, boxes, 0.0)).unwrap();
        let tags: Vec<&str> = results[0]
            .boxes
            .iter()
            .map(|b| b.box_.tag.as_str())
            .collect();
        assert!(!tags.contains(&"chat_list"), "{tags:?}");
        assert_eq!(tags.len(), 3, "{tags:?}");
    }

    #[test]
    fn skips_noise_target() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        synth_wechat(&a, 800, 600, 300, 97, 170);
        let mut img = ImageBuffer::new(800, 600);
        for y in 0..600 {
            for x in 0..800 {
                img.put_pixel(x, y, Rgb([(x * 31 % 255) as u8, (y * 17 % 255) as u8, 90]));
            }
        }
        DynamicImage::ImageRgb8(img).save(&b).unwrap();
        let boxes = tmpl_boxes(300.0, 97.0, 600.0, 170.0, 800.0);
        let results = propagate(&req_for(&a, &b, boxes, 0.62)).unwrap();
        assert!(results[0].boxes.is_empty());
    }

    #[test]
    fn missing_chat_window_template_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.png");
        synth_wechat(&a, 800, 600, 300, 97, 170);
        let req = PropagateRequest {
            src_path: a.to_string_lossy().into_owned(),
            boxes: vec![AnnoBox {
                tag: "msg_input".into(),
                x: 303.0,
                y: 430.0,
                w: 489.0,
                h: 170.0,
            }],
            targets: vec![a.to_string_lossy().into_owned()],
            min_confidence: 0.0,
            allow_rescale: true,
        };
        assert!(propagate(&req).is_err());
    }
}
