//! M0-C OCR 评测的 ROI 导出。
//!
//! 捕获自有测试窗口 → 结构定位 → 把标题与输入框 ROI 裁出，
//! 连同合成真值写入输出目录的 PNG 与 JSON 清单，供 Python 端 OCR 评测消费。
//!
//! 导出的全部内容来自自有合成窗口，不含任何真实聊天内容；
//! 输出目录只作为测试产物，列入 .gitignore，不进入版本库。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::RECT;

use crate::harness::chat_window::{self, Scene, Theme};
use crate::platform::capture::{capture_window_region, declare_dpi_awareness};
use crate::platform::clock::MonotonicClock;
use crate::platform::layout::{self, LayoutResult};
use crate::platform::window::foreground_snapshot;

/// 导出结果。
#[derive(Debug)]
pub struct ExportResult {
    pub manifest_path: PathBuf,
    pub cases: usize,
}

/// 把 BGRA 帧的一个矩形区域写成 PNG。
pub(crate) fn write_roi_png(
    path: &Path,
    frame: &crate::platform::capture::TargetFrame,
    roi: RECT,
) -> std::io::Result<()> {
    let x0 = roi.left.max(0) as u32;
    let y0 = roi.top.max(0) as u32;
    let x1 = roi.right.min(frame.width as i32).max(roi.left) as u32;
    let y1 = roi.bottom.min(frame.height as i32).max(roi.top) as u32;
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "空 ROI",
        ));
    }

    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;

    let stride = frame.stride as usize;
    let mut buffer = vec![0u8; (w * h * 4) as usize];
    let row_bytes = (w * 4) as usize;
    for (y, dst_row) in (y0..y1).zip(buffer.chunks_exact_mut(row_bytes)) {
        let start = y as usize * stride + x0 as usize * 4;
        let src = &frame.pixels[start..start + row_bytes];
        // BGRA → RGBA。
        for (dst, px) in dst_row.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            dst[0] = px[2];
            dst[1] = px[1];
            dst[2] = px[0];
            dst[3] = 255;
        }
    }
    writer.write_image_data(&buffer)?;
    Ok(())
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// 跑完整导出矩阵。输出目录为 `<repo>/artifacts/m0-ocr/`。
pub fn export_for_ocr() -> Result<ExportResult, String> {
    declare_dpi_awareness();
    let clock = MonotonicClock::new();

    let repo_root = std::env::current_dir().map_err(|e| e.to_string())?;
    let out_dir = repo_root.join("artifacts").join("m0-ocr");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    const SIZES: [(i32, i32); 4] = [(1000, 720), (1280, 820), (1440, 900), (860, 640)];
    let mut manifest = String::from("{\"cases\":[\n");
    let mut cases = 0usize;

    for theme in [Theme::Light, Theme::Dark] {
        for (idx, (w, h)) in SIZES.iter().enumerate() {
            let scene = Scene::sample(idx, theme);
            let label = format!(
                "{}-{}x{}#{}",
                match theme {
                    Theme::Light => "light",
                    Theme::Dark => "dark",
                },
                w,
                h,
                scene.title
            );

            let Some(hwnd) = chat_window::create_window(scene.clone(), *w, *h) else {
                return Err("无法创建测试窗口".to_string());
            };
            // 等待首次绘制与前台切换。
            std::thread::sleep(std::time::Duration::from_millis(350));
            crate::harness::layout_eval::pump_messages_public(std::time::Duration::from_millis(120));

            let result = (|| -> Result<(), String> {
                let snapshot =
                    foreground_snapshot().ok_or_else(|| "没有前台窗口".to_string())?;
                if snapshot.instance.root_hwnd != hwnd.0 as u64 {
                    return Err("测试窗口不是前台".to_string());
                }
                let frame = capture_window_region(snapshot.instance.root_hwnd, snapshot.client, &clock)
                    .map_err(|e| format!("捕获失败: {e:?}"))?;
                let detected: LayoutResult = layout::detect(
                    &frame.pixels,
                    frame.width as i32,
                    frame.height as i32,
                    frame.stride as usize,
                )
                .map_err(|e| format!("定位失败: {e:?}"))?;

                // 标题 ROI 适度内缩；草稿 ROI 只取输入区上半部，
                // 避免把输入区下缘的杂讯（如窗口控制元素残影）读进文本。
                let title_roi = expand(detected.chat_title, 4, 4);
                let compose_top = detected.compose_input.top;
                let compose_mid = compose_top + (detected.compose_input.bottom - compose_top) / 2;
                let compose_roi = RECT {
                    left: detected.compose_input.left,
                    top: compose_top + 2,
                    right: detected.compose_input.right,
                    bottom: compose_mid,
                };

                let title_png = out_dir.join(format!("{}.title.png", sanitize(&label)));
                let draft_png = out_dir.join(format!("{}.draft.png", sanitize(&label)));
                write_roi_png(&title_png, &frame, title_roi)
                    .map_err(|e| format!("写标题 PNG 失败: {e}"))?;
                write_roi_png(&draft_png, &frame, compose_roi)
                    .map_err(|e| format!("写草稿 PNG 失败: {e}"))?;

                if cases > 0 {
                    manifest.push_str(",\n");
                }
                let _ = write!(
                    manifest,
                    "  {{\"label\": \"{}\", \"title\": {{\"image\": \"{}\", \"expected\": \"{}\"}}, \"draft\": {{\"image\": \"{}\", \"expected\": \"{}\"}}}}",
                    json_escape(&label),
                    json_escape(&title_png.file_name().unwrap().to_string_lossy()),
                    json_escape(&scene.title),
                    json_escape(&draft_png.file_name().unwrap().to_string_lossy()),
                    json_escape(&scene.draft),
                );
                cases += 1;
                Ok(())
            })();

            chat_window::destroy_window(hwnd);
            crate::harness::layout_eval::pump_messages_public(std::time::Duration::from_millis(100));
            result?;
        }
    }

    manifest.push_str("\n]}\n");
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(&manifest_path, manifest).map_err(|e| e.to_string())?;

    Ok(ExportResult { manifest_path, cases })
}

fn sanitize(label: &str) -> String {
    label.replace('#', "-").replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_")
}

fn expand(rect: RECT, dx: i32, dy: i32) -> RECT {
    RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right - dx,
        bottom: rect.bottom - dy,
    }
}
