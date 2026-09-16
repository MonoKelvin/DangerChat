//! letterbox 预处理与坐标逆变换（§5.5 Preprocessor）。
//!
//! YOLO 输入要求固定边长（默认 640），letterbox 按长宽比缩放并居中填充，
//! 记录仿射参数供 postprocess 逆变换回原图坐标系。

use image::{imageops, RgbaImage};

use dc_sys::Rect;

/// 一次 letterbox 的几何参数：`(scale, pad_x, pad_y, dst_size)`。
/// 逆变换：`原图坐标 = (letterbox 坐标 - pad) / scale`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letterbox {
    pub scale: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub size: u32,
}

impl Letterbox {
    /// 计算几何参数（不缩放图像）。
    pub fn for_size(src_w: u32, src_h: u32, dst: u32) -> Self {
        let scale = (dst as f32 / src_w.max(1) as f32).min(dst as f32 / src_h.max(1) as f32);
        let new_w = (src_w as f32 * scale).round();
        let new_h = (src_h as f32 * scale).round();
        Self {
            scale,
            pad_x: ((dst as f32 - new_w) / 2.0).floor(),
            pad_y: ((dst as f32 - new_h) / 2.0).floor(),
            size: dst,
        }
    }

    /// letterbox 坐标 → 原图坐标。
    pub fn to_src(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.pad_x) / self.scale, (y - self.pad_y) / self.scale)
    }

    /// 原图坐标 → letterbox 坐标。
    pub fn to_dst(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.pad_x, y * self.scale + self.pad_y)
    }
}

/// RGBA → letterbox 后的 CHW f32 张量（RGB 顺序，0..1 归一化——YOLO 标准）。
///
/// 返回 `(数据, 几何参数)`。张量长度 = 3 × size × size。
pub fn letterbox_chw(src: &RgbaImage, size: u32) -> (Vec<f32>, Letterbox) {
    let geo = Letterbox::for_size(src.width(), src.height(), size);
    let new_w = ((src.width() as f32 * geo.scale).round() as u32).clamp(1, size);
    let new_h = ((src.height() as f32 * geo.scale).round() as u32).clamp(1, size);

    // resize 到缩放尺寸，再贴到 size×size 灰底（114 为 YOLO 惯例填充色）
    let mut canvas = image::RgbaImage::from_pixel(size, size, image::Rgba([114, 114, 114, 255]));
    let resized = imageops::resize(src, new_w, new_h, imageops::FilterType::Triangle);
    imageops::overlay(&mut canvas, &resized, geo.pad_x as i64, geo.pad_y as i64);

    // HWC RGBA → CHW f32（丢弃 alpha）
    let (w, h) = (size as usize, size as usize);
    let mut out = vec![0f32; 3 * w * h];
    for y in 0..h {
        for x in 0..w {
            let p = canvas.get_pixel(x as u32, y as u32);
            let i = y * w + x;
            out[i] = p[0] as f32 / 255.0;
            out[w * h + i] = p[1] as f32 / 255.0;
            out[2 * w * h + i] = p[2] as f32 / 255.0;
        }
    }
    (out, geo)
}

/// 检出框（letterbox 坐标系）→ 原图像素坐标 Rect，并 clamp 到图内。
pub fn rect_to_src(
    geo: &Letterbox,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    src_w: u32,
    src_h: u32,
) -> Rect {
    let (x1, y1) = geo.to_src(x, y);
    let (x2, y2) = geo.to_src(x + w, y + h);
    let x1 = x1.clamp(0.0, src_w as f32);
    let y1 = y1.clamp(0.0, src_h as f32);
    let x2 = x2.clamp(x1, src_w as f32);
    let y2 = y2.clamp(y1, src_h as f32);
    Rect::new(
        x1.round() as i32,
        y1.round() as i32,
        (x2 - x1).round().max(1.0) as u32,
        (y2 - y1).round().max(1.0) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UT-LAY-03：letterbox 坐标正逆变换往返误差 < 1px。
    #[test]
    fn ut_lay_03_letterbox_roundtrip() {
        for (w, h) in [(1068, 766), (530, 774), (1920, 1020), (640, 640)] {
            let geo = Letterbox::for_size(w, h, 640);
            for (x, y) in [
                (0.0, 0.0),
                (w as f32 / 2.0, h as f32 / 2.0),
                (w as f32 - 1.0, h as f32 - 1.0),
                (137.0, 251.0),
            ] {
                let (dx, dy) = geo.to_dst(x, y);
                let (bx, by) = geo.to_src(dx, dy);
                assert!(
                    (bx - x).abs() < 1.0 && (by - y).abs() < 1.0,
                    "{w}x{h} ({x},{y}): 逆变换漂移 ({:.2},{:.2})",
                    bx - x,
                    by - y
                );
            }
        }
    }

    /// rect 逆变换与 clamp：检出框不越界，尺寸保底 1px。
    #[test]
    fn rect_roundtrip_and_clamp() {
        let geo = Letterbox::for_size(1068, 766, 640);
        let r = rect_to_src(
            &geo,
            geo.pad_x,
            geo.pad_y,
            100.0 * geo.scale,
            50.0 * geo.scale,
            1068,
            766,
        );
        assert_eq!((r.x, r.y), (0, 0));
        assert_eq!((r.w, r.h), (100, 50));

        // 越界框 clamp 到图内
        let r = rect_to_src(&geo, -20.0, -20.0, 5000.0, 5000.0, 1068, 766);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert!(r.w <= 1068 && r.h <= 766);
        assert!(r.w >= 1 && r.h >= 1);
    }

    /// letterbox 张量形状与填充值。
    #[test]
    fn tensor_shape_and_padding() {
        // 200×100 → 640：scale=3.2，new=640×320，pad_y=160
        let img = RgbaImage::from_pixel(200, 100, image::Rgba([255, 0, 0, 255]));
        let (data, geo) = letterbox_chw(&img, 640);
        assert_eq!(data.len(), 3 * 640 * 640);
        assert_eq!(geo.pad_x, 0.0);
        // 缩放后图内一点（中心）应为红色：R=1 G=0 B=0
        let idx = |c: usize, y: usize, x: usize| c * 640 * 640 + y * 640 + x;
        assert!((data[idx(0, 160, 320)] - 1.0).abs() < 1e-3, "R 通道");
        assert!(data[idx(1, 160, 320)].abs() < 1e-3, "G 通道");
        // 上边 padding 应为 114/255
        assert!(
            (data[idx(0, 80, 320)] - 114.0 / 255.0).abs() < 1e-3,
            "padding 值"
        );
    }
}
