//! UT-OCR-01 黄金集测试（§11.1 独立层）：20 张黄金切片 → 字段级准确率 ≥ 95%。
//!
//! 与 `ocr.rs`（纯逻辑/桩权重，快跑层）分开的**独立 test 目标**：本文件全部
//! 测试需要 models/ 下的真实 OCR 三件套权重（gitignore，按 models/README
//! 下载）。缺权重时 skip 并提示，本地备好权重后自动全跑。
//!
//! 快跑路径（`pnpm test:fast`）按 binary(golden) 过滤排除本目标；
//! 全量（`pnpm test`）与黄金层（`pnpm test:golden`）会执行。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use dc_core::{ConfigSnapshot, ConfigValue, ImageLogSink, ModelStore, ModuleContext, RunId};
use dc_pipeline::contract::{LoopKind, PipelineContext, Region, RegionLayout};
use dc_pipeline::{Module, OcrStage, Stage, WindowSnapshot};
use dc_sys::Rect;

const SLICES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ocr_slices");
/// 模型根：仓库根的 models/（三件套 + 字典）。
const MODELS_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../models");

fn models_ready() -> bool {
    let root = Path::new(MODELS_ROOT);
    root.join("ocr-det-ppocrv4/ch_PP-OCRv4_det_mobile.onnx")
        .is_file()
        && root
            .join("ocr-cls-ppocrv20/ch_ppocr_mobile_v2.0_cls_mobile.onnx")
            .is_file()
        && root
            .join("ocr-rec-ppocrv4/ch_PP-OCRv4_rec_mobile.onnx")
            .is_file()
        && root.join("ocr-rec-ppocrv4/ppocr_keys_v1.txt").is_file()
}

fn module_ctx() -> ModuleContext {
    let mut values = BTreeMap::new();
    values.insert("device.prefer_gpu".to_string(), ConfigValue::Bool(false));
    ModuleContext {
        config: Arc::new(ConfigSnapshot::new(values, 1)),
        models: Arc::new(ModelStore::new(MODELS_ROOT)),
        log_dir: std::env::temp_dir(),
    }
}

fn run_ctx(kind: LoopKind) -> PipelineContext {
    PipelineContext::new(
        RunId::from_raw("20260913-210000-000001"),
        kind,
        ImageLogSink::new(std::env::temp_dir(), RunId::from_raw("x"), false),
        Arc::new(ConfigSnapshot::default()),
    )
}

/// 把一张切片包成「窗口快照 + 全图 msg_input 布局」喂给 Stage。
fn slice_input(path: &Path) -> (WindowSnapshot, RegionLayout) {
    let img = image::open(path).expect("切片读取失败").to_rgba8();
    let (w, h) = (img.width(), img.height());
    (
        WindowSnapshot {
            image: img,
            window_rect: Rect::new(0, 0, w, h),
            dpi_scale: 1.0,
            captured_at: Instant::now(),
        },
        RegionLayout {
            regions: vec![Region {
                tag: "msg_input".into(),
                rect: Rect::new(0, 0, w, h),
                confidence: 1.0,
            }],
            layout_epoch: 0,
            inferred_at: Instant::now(),
        },
    )
}

/// UT-OCR-01：20 张黄金切片，字段级准确率 ≥ 95%。
///
/// 字段级 = 每张切片的 draft_text 与期望**完全相等**记 1 分（噪声词已过滤、
/// 行序已归一）；空输入框期望空串。95% 门禁 = 至多 1 张不匹配。
#[test]
fn ut_ocr_01_golden_slices_accuracy() {
    if !models_ready() {
        eprintln!("跳过 UT-OCR-01：models/ 下 OCR 三件套不齐（按 models/README.md 下载）");
        return;
    }
    let mctx = module_ctx();
    let mut stage = OcrStage::new();
    stage.init(&mctx).expect("OCR init 失败");

    let expected: std::collections::HashMap<String, String> = {
        let text = std::fs::read_to_string(format!("{SLICES}/expected.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    };

    let mut files: Vec<_> = std::fs::read_dir(SLICES)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    assert!(files.len() >= 20, "黄金切片不足 20 张：{}", files.len());

    let ctx = run_ctx(LoopKind::Fast);
    let mut matched = 0usize;
    let mut misses: Vec<String> = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let out = stage.process(slice_input(f), &ctx).expect("OCR 推理失败");
        let want = expected.get(&name).cloned().unwrap_or_default();
        if out.draft_text == want {
            matched += 1;
        } else {
            misses.push(format!(
                "{name}:\n  got  {:?}\n  want {:?}",
                out.draft_text, want
            ));
        }
    }
    let acc = matched as f64 / files.len() as f64;
    assert!(
        acc >= 0.95,
        "字段级准确率 {:.0}%（{}/{}）低于 95%：\n{}",
        acc * 100.0,
        matched,
        files.len(),
        misses.join("\n")
    );
}

/// ROI 空/过小 → 对应字段空值（非 Err）（错误矩阵）。需要真实权重。
#[test]
fn tiny_roi_yields_empty_not_err() {
    if !models_ready() {
        eprintln!("跳过：OCR 权重不齐");
        return;
    }
    let mctx = module_ctx();
    let mut stage = OcrStage::new();
    stage.init(&mctx).unwrap();

    // msg_input 检出矩形只有 2×2（过小）→ draft_text 空串
    let img = image::RgbaImage::new(100, 100);
    let input = (
        WindowSnapshot {
            image: img,
            window_rect: Rect::new(0, 0, 100, 100),
            dpi_scale: 1.0,
            captured_at: Instant::now(),
        },
        RegionLayout {
            regions: vec![Region {
                tag: "msg_input".into(),
                rect: Rect::new(10, 10, 2, 2),
                confidence: 0.9,
            }],
            layout_epoch: 0,
            inferred_at: Instant::now(),
        },
    );
    let out = stage.process(input, &run_ctx(LoopKind::Fast)).unwrap();
    assert_eq!(out.draft_text, "");
    assert_eq!(out.chat_target, None);
}
