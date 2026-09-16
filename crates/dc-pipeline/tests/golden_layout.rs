//! UT-LAY-01 黄金集测试（§11.1 独立层）：黄金图 × 3 → 四 tag 检出、IoU ≥ 0.7。
//!
//! 与 `layout.rs`（纯逻辑/桩权重，快跑层）分开的**独立 test 目标**：本文件
//! 需要 models/layout-wechat 自训练权重（gitignore，tools/training/train_layout.py）。
//! 缺权重时 skip 并提示，本地备好权重后自动全跑。
//!
//! 快跑路径（`pnpm test:fast`）按 binary(golden) 过滤排除本目标；
//! 全量（`pnpm test`）与黄金层（`pnpm test:golden`）会执行。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use dc_core::{ConfigValue, ImageLogSink, ModuleContext, RunId};
use dc_pipeline::contract::{LoopKind, PipelineContext};
use dc_pipeline::{LayoutStage, Module, Stage, WindowSnapshot};
use dc_sys::Rect;

fn run_ctx() -> PipelineContext {
    PipelineContext::new(
        RunId::from_raw("20260913-200000-000001"),
        LoopKind::Slow,
        ImageLogSink::new(
            std::path::PathBuf::from("unused"),
            RunId::from_raw("x"),
            false,
        ),
        Arc::new(dc_core::ConfigSnapshot::default()),
    )
}

/// UT-LAY-01：黄金图 × 3 → 四 tag 检出、IoU ≥ 0.7（需权重 models/dc-layout-wechat）。
#[test]
fn ut_lay_01_golden_regions_iou() {
    /// 模型根：仓库根的 models/。
    const MODELS_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../models");
    let model_dir = Path::new(MODELS_ROOT).join("dc-layout-wechat");
    if !model_dir.join("model.toml").is_file() {
        eprintln!("跳过 UT-LAY-01：models/dc-layout-wechat 缺失（tools/training/train_layout.py）");
        return;
    }

    // ModuleContext 指向仓库根 models/，layout.model = dc-layout-wechat
    let tmp = tempfile::tempdir().unwrap();
    let mut values = BTreeMap::new();
    values.insert(
        "layout.model".to_string(),
        ConfigValue::Str("dc-layout-wechat".into()),
    );
    values.insert("device.prefer_gpu".to_string(), ConfigValue::Bool(false));
    // 34 张训练的模型置信度未校准（正确框可低至 ~0.16）；黄金测试验证几何正确性，
    // 阈值压低到 0.15。标满 73 张重训后应回归默认并重新校准。
    values.insert(
        "layout.conf_threshold".to_string(),
        ConfigValue::Float(0.15),
    );
    let mctx = ModuleContext {
        config: Arc::new(dc_core::ConfigSnapshot::new(values, 1)),
        models: Arc::new(dc_core::ModelStore::new(MODELS_ROOT)),
        log_dir: tmp.path().to_path_buf(),
    };
    let mut stage = LayoutStage::new();
    stage.init(&mctx).expect("layout init 失败（模型非法？）");

    let golden_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden");
    let expected: std::collections::HashMap<String, serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string(golden_dir.join("expected_regions.json")).unwrap(),
    )
    .unwrap();

    let ctx = run_ctx();
    for (name, exp) in &expected {
        let img = image::open(golden_dir.join(name)).unwrap().to_rgba8();
        let snap = WindowSnapshot {
            image: img,
            window_rect: Rect::new(
                0,
                0,
                exp["size"][0].as_u64().unwrap() as u32,
                exp["size"][1].as_u64().unwrap() as u32,
            ),
            dpi_scale: 1.0,
            captured_at: Instant::now(),
        };
        let out = stage
            .process(snap, &ctx)
            .unwrap_or_else(|e| panic!("{name} 推理失败：{e}"));

        for region in exp["regions"].as_array().unwrap() {
            let tag = region["tag"].as_str().unwrap();
            let (x, y, w, h) = (
                region["rect"][0].as_i64().unwrap() as i32,
                region["rect"][1].as_i64().unwrap() as i32,
                region["rect"][2].as_i64().unwrap() as u32,
                region["rect"][3].as_i64().unwrap() as u32,
            );
            let got = out.rect_of(tag).unwrap_or_else(|| {
                panic!(
                    "{name}：未检出 {tag}（检出的 tag：{:?}）",
                    out.regions
                        .iter()
                        .map(|r| r.tag.as_str())
                        .collect::<Vec<_>>()
                )
            });
            let overlap = got.x.max(x)..(got.x + got.w as i32).min(x + w as i32);
            let overlap_y = got.y.max(y)..(got.y + got.h as i32).min(y + h as i32);
            let inter = (overlap.len() * overlap_y.len()) as f64;
            let union = (got.w as f64 * got.h as f64) + (w as f64 * h as f64) - inter;
            let iou = inter / union;
            assert!(
                iou >= 0.7,
                "{name}：{tag} IoU {iou:.2} < 0.7（got {:?} want ({x},{y},{w},{h})）",
                (got.x, got.y, got.w, got.h)
            );
        }
    }
}
