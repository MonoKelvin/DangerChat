//! pHash 感知哈希（§2.3 慢环心跳）：32×32 灰度 → DCT-II → 8×8 低频 → 64bit。
//!
//! 用于 chat_target ROI 的「像素是否实质变化」判定——海明距 ≤4 视为未变
//! （抗轻微渲染抖动/抗锯齿差异），GIF 动画等其余区域像素变化不进哈希范围。

const SIZE: usize = 32;
const LOW: usize = 8;

/// 计算感知哈希（64bit）。
/// DCT 循环的变量即频率坐标（数值算法常态），豁免 range-loop lint。
#[allow(clippy::needless_range_loop)]
pub fn phash(img: &image::RgbaImage) -> u64 {
    // 1) 缩到 32×32 灰度
    let small = image::imageops::resize(
        img,
        SIZE as u32,
        SIZE as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut gray = [[0f32; SIZE]; SIZE];
    for (x, y, p) in small.enumerate_pixels() {
        // BT.601 luma（0..255）
        gray[y as usize][x as usize] =
            0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
    }

    // 2) 可分离 DCT-II：先行后列，只留左上 8×8 低频块
    let mut rows = [[0f32; SIZE]; SIZE];
    for y in 0..SIZE {
        for u in 0..LOW {
            let mut sum = 0f32;
            for x in 0..SIZE {
                sum += gray[y][x]
                    * (std::f32::consts::PI * (2 * x + 1) as f32 * u as f32 / (2.0 * SIZE as f32))
                        .cos();
            }
            rows[y][u] = sum;
        }
    }
    let mut dct = [[0f32; LOW]; LOW];
    for u in 0..LOW {
        for v in 0..LOW {
            let mut sum = 0f32;
            for y in 0..SIZE {
                sum += rows[y][u]
                    * (std::f32::consts::PI * (2 * y + 1) as f32 * v as f32 / (2.0 * SIZE as f32))
                        .cos();
            }
            dct[v][u] = sum;
        }
    }

    // 3) 去 DC（[0][0]）后取中值阈值 → 63bit 有效（第 0 位恒 0）
    let mut values = Vec::with_capacity(LOW * LOW - 1);
    for v in 0..LOW {
        for u in 0..LOW {
            if !(u == 0 && v == 0) {
                values.push(dct[v][u]);
            }
        }
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = values[values.len() / 2];

    let mut hash: u64 = 0;
    let mut bit = 1u64;
    for v in 0..LOW {
        for u in 0..LOW {
            if !(u == 0 && v == 0) {
                if dct[v][u] > median {
                    hash |= bit;
                }
                bit <<= 1;
            }
        }
    }
    hash
}

/// 海明距。
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 心跳判定：海明距 ≤4 视为「像素未实质变化」。
pub fn unchanged(a: u64, b: u64) -> bool {
    hamming(a, b) <= 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(color: [u8; 4]) -> image::RgbaImage {
        image::RgbaImage::from_pixel(200, 100, Rgba(color))
    }

    /// 同图哈希恒等；纯色图间互异（低频结构不同）。
    #[test]
    fn hash_identity_and_difference() {
        let a = solid([200, 50, 50, 255]);
        let h1 = phash(&a);
        let h2 = phash(&a);
        assert_eq!(h1, h2, "同图哈希应确定");

        let b = solid([50, 200, 50, 255]);
        assert!(!unchanged(h1, phash(&b)), "不同颜色（灰度不同）应判变化");
    }

    /// 微小扰动（两个像素）不改变判定（抗抖动）。
    ///
    /// 底图带渐变纹理——纯色图是退化输入（中值阈值在噪声级系数上翻面），
    /// 真实聊天区截图总有文字纹理，不退化。
    #[test]
    fn tiny_perturbation_unchanged() {
        let mut img = image::RgbaImage::new(200, 100);
        for y in 0..img.height() {
            for x in 0..img.width() {
                let v = (x * 3 / 2 + y / 2) as u8;
                img.put_pixel(x, y, Rgba([v, v, v, 255]));
            }
        }
        let h1 = phash(&img);
        img.put_pixel(5, 5, Rgba([255, 255, 255, 255]));
        img.put_pixel(100, 40, Rgba([0, 0, 0, 255]));
        let h2 = phash(&img);
        assert!(
            unchanged(h1, h2),
            "2/20000 像素变化应判未变（dist={}）",
            hamming(h1, h2)
        );
    }

    /// 大块变化判变化（聊天对象切换的模拟）。
    #[test]
    fn structural_change_detected() {
        let mut img = solid([100, 100, 100, 255]);
        // 左半屏换成另一种灰度 → 低频结构大变
        for y in 0..img.height() {
            for x in 0..img.width() / 2 {
                img.put_pixel(x, y, Rgba([220, 220, 220, 255]));
            }
        }
        let h1 = phash(&solid([100, 100, 100, 255]));
        let h2 = phash(&img);
        assert!(
            !unchanged(h1, h2),
            "半屏变化应判变化（dist={}）",
            hamming(h1, h2)
        );
    }
}
