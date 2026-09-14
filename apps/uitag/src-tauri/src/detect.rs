use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rayon::prelude::*;
use serde::Serialize;

/// 识别出的吸附线：orient = "h"（水平线，pos 为 y）| "v"（垂直线，pos 为 x），
/// start/end 为线在另一轴上的跨度（图像像素坐标）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SnapLine {
    pub orient: String,
    pub pos: u32,
    pub start: u32,
    pub end: u32,
    /// 差异段占全宽/全高比例（0–1），越大越显著
    pub strength: f64,
}

type CacheKey = (String, u32);

/// 识别结果缓存：key = (图片路径, 最小连续像素)。当前图异步识别完成后命中缓存直接返回。
static CACHE: Mutex<Option<HashMap<CacheKey, Arc<Vec<SnapLine>>>>> = Mutex::new(None);

/// 像素差阈值：RGB 曼哈顿距离（0–765），约每通道 4——取低值捕捉 1px 浅色边框
/// （如 #f4f4f4 底上的 #e8e8e8 边线，差 18）。截图为无损 PNG 噪声小，
/// 零星误报由 min_run 连续长度过滤；缓变渐变（<4/通道/行）不会触发。
const COLOR_TOL: i32 = 12;
/// 相邻候选行/列聚类半径（像素）：抗锯齿边缘常在 1–2 行内都出现差异
const MERGE_RADIUS: u32 = 2;
/// 差异段内允许的最大间隙（像素）：面板边界被小图标/文字打断时仍视为同一条线
const MAX_GAP: u32 = 12;

/// 检测图片中的水平/垂直分割线：
/// 对每对相邻行（列）逐像素比较颜色距离，连续差异段长度 ≥ min_run 即认为
/// 存在一条分割线（同色横线本身、或两侧异色的区域边界，两种情况统一覆盖）。
pub fn detect(path: &str, min_run: u32) -> Result<Vec<SnapLine>, String> {
    let key = (path.to_string(), min_run);
    {
        let guard = CACHE.lock().unwrap();
        if let Some(map) = guard.as_ref() {
            if let Some(hit) = map.get(&key) {
                return Ok(hit.as_ref().clone());
            }
        }
    }

    let img = image::open(path)
        .map_err(|e| format!("读取图片失败：{e}"))?
        .to_rgb8();
    let (w, h) = (img.width(), img.height());
    let buf = img.as_raw();
    let mut lines = Vec::new();
    if w >= 2 && h >= 2 {
        lines.extend(scan_horizontal(buf, w, h, min_run));
        lines.extend(scan_vertical(buf, w, h, min_run));
    }

    let out = Arc::new(lines);
    let mut guard = CACHE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    // 简单防膨胀：只保留最近 32 组结果
    if map.len() >= 32 {
        map.clear();
    }
    map.insert(key, out.clone());
    Ok(out.as_ref().clone())
}

/// 扫描一对相邻基线的差异段：返回 (差异像素数, 起点索引, 终点索引)。
/// 闭区间；差异段内允许 ≤ MAX_GAP 的间隙（图标/文字打断仍连续）。
/// diff_fn(i) = 位置 i 处是否存在显著颜色差异。
fn best_run(len: usize, min_run: u32, diff_fn: impl Fn(usize) -> bool) -> Option<(u32, usize, usize)> {
    let mut best: Option<(u32, usize, usize)> = None;
    let mut x0 = 0usize;
    let mut diff_count = 0u32;
    let mut last_diff = 0usize; // 最近一个差异像素位置
    let mut gap = 0u32;
    let mut in_run = false;
    let update_best = |best: &mut Option<(u32, usize, usize)>, c: u32, s: usize, e: usize| {
        if c >= min_run && best.map_or(true, |b| c > b.0) {
            *best = Some((c, s, e));
        }
    };
    for i in 0..len {
        if diff_fn(i) {
            if !in_run {
                in_run = true;
                x0 = i;
                diff_count = 0;
            }
            diff_count += 1;
            last_diff = i;
            gap = 0;
        } else if in_run {
            gap += 1;
            if gap > MAX_GAP {
                update_best(&mut best, diff_count, x0, last_diff);
                in_run = false;
            }
        }
    }
    if in_run {
        update_best(&mut best, diff_count, x0, last_diff);
    }
    best
}

fn scan_horizontal(buf: &[u8], w: u32, h: u32, min_run: u32) -> Vec<SnapLine> {
    let (w, h) = (w as usize, h as usize);
    let px = |x: usize, y: usize| (y * w + x) * 3;
    // 行间独立，rayon 并行扫描
    let candidates: Vec<SnapLine> = (0..h.saturating_sub(1))
        .into_par_iter()
        .filter_map(|y| {
            best_run(w, min_run, |x| {
                let a = px(x, y);
                let b = px(x, y + 1);
                let d = (buf[a] as i32 - buf[b] as i32).abs()
                    + (buf[a + 1] as i32 - buf[b + 1] as i32).abs()
                    + (buf[a + 2] as i32 - buf[b + 2] as i32).abs();
                d > COLOR_TOL
            })
            .map(|(count, x0, x1)| SnapLine {
                orient: "h".into(),
                pos: y as u32,
                start: x0 as u32,
                end: x1 as u32,
                strength: count as f64 / w as f64,
            })
        })
        .collect();
    merge_adjacent(candidates)
}

fn scan_vertical(buf: &[u8], w: u32, h: u32, min_run: u32) -> Vec<SnapLine> {
    let (w, h) = (w as usize, h as usize);
    let px = |x: usize, y: usize| (y * w + x) * 3;
    let candidates: Vec<SnapLine> = (0..w.saturating_sub(1))
        .into_par_iter()
        .filter_map(|x| {
            best_run(h, min_run, |y| {
                let a = px(x, y);
                let b = px(x + 1, y);
                let d = (buf[a] as i32 - buf[b] as i32).abs()
                    + (buf[a + 1] as i32 - buf[b + 1] as i32).abs()
                    + (buf[a + 2] as i32 - buf[b + 2] as i32).abs();
                d > COLOR_TOL
            })
            .map(|(count, y0, y1)| SnapLine {
                orient: "v".into(),
                pos: x as u32,
                start: y0 as u32,
                end: y1 as u32,
                strength: count as f64 / h as f64,
            })
        })
        .collect();
    merge_adjacent(candidates)
}

/// 把半径 MERGE_RADIUS 内的相邻候选合并为一条：保留强度更高的位置，跨度取并集
/// （抗锯齿边缘会在相邻 1–2 行重复出现候选）。
fn merge_adjacent(mut lines: Vec<SnapLine>) -> Vec<SnapLine> {
    lines.sort_by_key(|l| l.pos);
    let mut out: Vec<SnapLine> = Vec::new();
    for l in lines {
        match out.last_mut() {
            Some(prev) if l.pos <= prev.pos + MERGE_RADIUS => {
                if l.strength > prev.strength {
                    prev.pos = l.pos;
                    prev.strength = l.strength;
                }
                prev.start = prev.start.min(l.start);
                prev.end = prev.end.max(l.end);
            }
            _ => out.push(l),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, fill: impl Fn(u32, u32) -> [u8; 3]) -> (Vec<u8>, u32, u32) {
        let mut buf = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                buf.extend_from_slice(&fill(x, y));
            }
        }
        (buf, w, h)
    }

    /// 区域边界：上半白下半灰 → y=49 处一条水平线
    #[test]
    fn detects_region_boundary() {
        let (buf, w, h) = solid(200, 100, |_, y| {
            if y < 50 { [255, 255, 255] } else { [120, 120, 120] }
        });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].pos, 49);
        assert_eq!(lines[0].start, 0);
        assert_eq!(lines[0].end, 199);
    }

    /// 同色细线：白底中 1px 灰线 → 线附近一条水平线（相邻候选合并）
    #[test]
    fn detects_solid_thin_line() {
        let (buf, w, h) = solid(200, 100, |_, y| if y == 50 { [120, 120, 120] } else { [255, 255, 255] });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!((49..=50).contains(&lines[0].pos));
    }

    /// 连续长度不足 min_run 的差异不算线
    #[test]
    fn ignores_short_runs() {
        let (buf, w, h) = solid(200, 100, |x, y| {
            if y < 50 || x < 150 { [255, 255, 255] } else { [120, 120, 120] }
        });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// 垂直区域边界
    #[test]
    fn detects_vertical_boundary() {
        let (buf, w, h) = solid(100, 200, |x, _| {
            if x < 40 { [255, 255, 255] } else { [90, 90, 90] }
        });
        let lines = scan_vertical(&buf, w, h, 100);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].pos, 39);
    }

    /// 小噪声（每通道 ≤3）不触发；缓变渐变不会产生假线
    #[test]
    fn ignores_small_noise() {
        let (buf, w, h) = solid(200, 100, |_, y| if y < 50 { [255, 255, 255] } else { [252, 252, 252] });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// 1px 浅色边框线（与背景差 6/通道）也能识别为一条线
    #[test]
    fn detects_subtle_1px_border() {
        let (buf, w, h) = solid(200, 100, |_, y| {
            if y == 50 { [232, 232, 232] } else { [244, 244, 244] }
        });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!((49..=50).contains(&lines[0].pos));
    }

    /// 浅色面板色差（#fff vs #f2f2f2）也能识别
    #[test]
    fn detects_subtle_panel_difference() {
        let (buf, w, h) = solid(200, 100, |_, y| {
            if y < 50 { [255, 255, 255] } else { [242, 242, 242] }
        });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert_eq!(lines.len(), 1, "{lines:?}");
    }

    /// 边界被小图标打断（≤12px 间隙）仍视为同一条线，跨度横跨间隙
    #[test]
    fn gap_tolerant_run() {
        let (buf, w, h) = solid(200, 100, |x, y| {
            if y < 50 {
                [255, 255, 255]
            } else if y == 50 && (90..96).contains(&x) {
                [255, 255, 255] // 模拟打断边界的 6px 图标
            } else {
                [120, 120, 120]
            }
        });
        let lines = scan_horizontal(&buf, w, h, 100);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].start, 0);
        assert_eq!(lines[0].end, 199);
    }
}
