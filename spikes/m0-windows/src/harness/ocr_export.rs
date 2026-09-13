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
use crate::platform::window::snapshot_window;

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
    std::fs::write(path, encode_roi_png(frame, roi)?)
}

/// 把 ROI 编码到内存。拒绝部分越界，避免裁掉内容后继续产生安全结论。
pub(crate) fn encode_roi_png(
    frame: &crate::platform::capture::TargetFrame,
    roi: RECT,
) -> std::io::Result<Vec<u8>> {
    let stride = frame.stride as usize;
    if roi.left < 0
        || roi.top < 0
        || roi.right <= roi.left
        || roi.bottom <= roi.top
        || roi.right as u32 > frame.width
        || roi.bottom as u32 > frame.height
        || u64::from(frame.stride) < u64::from(frame.width) * 4
        || u64::from(frame.stride) * u64::from(frame.height) > frame.pixels.len() as u64
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ROI 或像素缓冲区无效",
        ));
    }
    let x0 = roi.left as u32;
    let y0 = roi.top as u32;
    let x1 = roi.right as u32;
    let y1 = roi.bottom as u32;
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "空 ROI",
        ));
    }

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;

    let mut buffer = vec![0u8; (w * h * 4) as usize];
    let row_bytes = (w * 4) as usize;
    for (y, dst_row) in (y0..y1).zip(buffer.chunks_exact_mut(row_bytes)) {
        let start = y as usize * stride + x0 as usize * 4;
        let src = &frame.pixels[start..start + row_bytes];
        // BGRA → RGBA。
        for (dst, px) in dst_row
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(src.as_chunks::<4>().0)
        {
            *dst = [px[2], px[1], px[0], 255];
        }
    }
    writer.write_image_data(&buffer)?;
    writer.finish()?;
    Ok(png)
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
            // 等待窗口映射到屏幕；绘制完成在下面显式确认。
            crate::harness::layout_eval::pump_messages_public(std::time::Duration::from_millis(
                120,
            ));

            let result = (|| -> Result<(), String> {
                if !chat_window::wait_for_repaint(hwnd, std::time::Duration::from_secs(2)) {
                    return Err("测试窗口未完成绘制".to_string());
                }
                let snapshot =
                    snapshot_window(hwnd).ok_or_else(|| "无法读取测试窗口元数据".to_string())?;
                if snapshot.instance.root_hwnd != hwnd.0 as u64
                    || !snapshot.visible
                    || snapshot.minimized
                {
                    return Err("测试窗口不可捕获".to_string());
                }
                let frame =
                    capture_window_region(snapshot.instance.root_hwnd, snapshot.client, &clock)
                        .map_err(|e| format!("捕获失败: {e:?}"))?;
                let detected: LayoutResult = layout::detect(
                    &frame.pixels,
                    frame.width as i32,
                    frame.height as i32,
                    frame.stride as usize,
                )
                .map_err(|e| format!("定位失败: {e:?}"))?;

                // 标题与草稿保留完整区域，仅避开边框。
                let title_roi = expand(detected.chat_title, 4, 4);
                let compose_roi = expand(detected.compose_input, 4, 4);

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
            crate::harness::layout_eval::pump_messages_public(std::time::Duration::from_millis(
                100,
            ));
            result?;
        }
    }

    manifest.push_str("\n]}\n");
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(&manifest_path, manifest).map_err(|e| e.to_string())?;

    Ok(ExportResult {
        manifest_path,
        cases,
    })
}

fn sanitize(label: &str) -> String {
    label
        .replace('#', "-")
        .replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_")
}

fn expand(rect: RECT, dx: i32, dy: i32) -> RECT {
    RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right - dx,
        bottom: rect.bottom - dy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_manifest_strings() {
        assert_eq!(json_escape("a\\\"\n"), "a\\\\\\\"\\n");
    }

    #[test]
    fn sanitizes_windows_file_name_characters() {
        assert_eq!(sanitize("light#x:y/a"), "light-x_y_a");
    }

    #[test]
    fn inset_keeps_expected_bounds() {
        let rect = RECT {
            left: 10,
            top: 20,
            right: 110,
            bottom: 220,
        };
        let actual = expand(rect, 4, 6);
        assert_eq!((actual.left, actual.top), (14, 26));
        assert_eq!((actual.right, actual.bottom), (106, 214));
    }
}
