use crate::state::AnnoBox;

/// 像素矩形 → YOLO 归一化行 `class cx cy w h`（均除以图宽/高，保留 6 位小数）。
/// class 为 None（框的 tag 不在 tags 表中）时返回 None——调用方应保证 tag 合法。
pub fn to_yolo_line(b: &AnnoBox, img_w: u32, img_h: u32, class_id: usize) -> Option<String> {
    if img_w == 0 || img_h == 0 {
        return None;
    }
    let (w, h) = (img_w as f64, img_h as f64);
    // 中心点与宽高
    let cx = (b.x + b.w / 2.0) / w;
    let cy = (b.y + b.h / 2.0) / h;
    let nw = b.w / w;
    let nh = b.h / h;
    // 防御：clamp 到 [0,1]，避免手抖拖出画布产生越界标注
    let (cx, cy) = (cx.clamp(0.0, 1.0), cy.clamp(0.0, 1.0));
    let (nw, nh) = (nw.clamp(0.0, 1.0), nh.clamp(0.0, 1.0));
    // 10 位小数：6 位对 ~4000px 宽图往返误差可达 ~2e-4，压不进 UT-TAG-01 的 1e-4 容差
    Some(format!("{class_id} {cx:.10} {cy:.10} {nw:.10} {nh:.10}"))
}

/// YOLO 归一化行 → 像素矩形（UT-TAG-01 的往返路径）。
pub fn from_yolo_line(line: &str, img_w: u32, img_h: u32, tag: &str) -> Option<AnnoBox> {
    let (w, h) = (img_w as f64, img_h as f64);
    let mut it = line.split_whitespace();
    let _class = it.next()?;
    let cx: f64 = it.next()?.parse().ok()?;
    let cy: f64 = it.next()?.parse().ok()?;
    let nw: f64 = it.next()?.parse().ok()?;
    let nh: f64 = it.next()?.parse().ok()?;
    Some(AnnoBox {
        tag: tag.to_string(),
        x: (cx * w) - (nw * w) / 2.0,
        y: (cy * h) - (nh * h) / 2.0,
        w: nw * w,
        h: nh * h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UT-TAG-01：像素 → YOLO 归一化 → 像素往返误差 < 1e-4。
    /// 覆盖：常规值、奇数尺寸（530×774）、零偏移原点、边界贴边。
    #[test]
    fn ut_tag_01_pixel_yolo_roundtrip() {
        let cases: Vec<(u32, u32, AnnoBox)> = vec![
            (
                1068,
                766,
                AnnoBox {
                    tag: "a".into(),
                    x: 100.0,
                    y: 50.0,
                    w: 300.0,
                    h: 200.0,
                },
            ),
            (
                530,
                774,
                AnnoBox {
                    tag: "a".into(),
                    x: 13.0,
                    y: 7.0,
                    w: 127.0,
                    h: 251.0,
                },
            ),
            (
                530,
                774,
                AnnoBox {
                    tag: "a".into(),
                    x: 0.0,
                    y: 0.0,
                    w: 530.0,
                    h: 774.0,
                },
            ),
            (
                1920,
                1080,
                AnnoBox {
                    tag: "a".into(),
                    x: 0.5,
                    y: 0.25,
                    w: 1.0,
                    h: 2.0,
                },
            ),
        ];
        for (iw, ih, b) in cases {
            let line = to_yolo_line(&b, iw, ih, 0).unwrap();
            let back = from_yolo_line(&line, iw, ih, &b.tag).unwrap();
            assert!(
                (back.x - b.x).abs() < 1e-4,
                "x drift on {iw}x{ih}: {}",
                back.x - b.x
            );
            assert!((back.y - b.y).abs() < 1e-4, "y drift on {iw}x{ih}");
            assert!((back.w - b.w).abs() < 1e-4, "w drift on {iw}x{ih}");
            assert!((back.h - b.h).abs() < 1e-4, "h drift on {iw}x{ih}");
        }
    }

    #[test]
    fn yolo_line_format() {
        let b = AnnoBox {
            tag: "t".into(),
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        let line = to_yolo_line(&b, 200, 100, 2).unwrap();
        assert_eq!(
            line,
            "2 0.2500000000 0.2500000000 0.5000000000 0.5000000000"
        );
    }

    #[test]
    fn zero_size_image_returns_none() {
        let b = AnnoBox {
            tag: "t".into(),
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
        assert!(to_yolo_line(&b, 0, 100, 0).is_none());
    }
}
