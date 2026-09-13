//! M4 layout 模块测试（UT-LAY-02~06；UT-LAY-01 黄金图 IoU 等自训练权重，标完 73 张后启用）。

use dc_core::{ModelMeta, ModelStore};
use dc_pipeline::layout::postprocess::{decode, map_tags, nms, RawDet};
use dc_pipeline::layout::preprocess::{letterbox_chw, rect_to_src, Letterbox};
use image::RgbaImage;

// ---------------------------------------------------------------------------
// UT-LAY-02/03：纯函数层（源码内单测覆盖更细，这里走公共 API 冒烟）
// ---------------------------------------------------------------------------

#[test]
fn ut_lay_02_nms_overlapping_suppressed() {
    let dets = vec![
        RawDet { x: 100.0, y: 100.0, w: 50.0, h: 50.0, class: 0, score: 0.9 },
        RawDet { x: 102.0, y: 101.0, w: 50.0, h: 50.0, class: 0, score: 0.8 },
        RawDet { x: 400.0, y: 100.0, w: 50.0, h: 50.0, class: 0, score: 0.7 },
        RawDet { x: 100.0, y: 100.0, w: 60.0, h: 60.0, class: 1, score: 0.6 },
    ];
    let kept = nms(&dets, 0.5);
    assert_eq!(kept.len(), 3);
}

#[test]
fn ut_lay_03_letterbox_roundtrip_under_1px() {
    let geo = Letterbox::for_size(1068, 766, 640);
    let (dx, dy) = geo.to_dst(500.0, 300.0);
    let (bx, by) = geo.to_src(dx, dy);
    assert!((bx - 500.0).abs() < 1.0 && (by - 300.0).abs() < 1.0);

    let img = RgbaImage::from_pixel(100, 200, image::Rgba([10, 20, 30, 255]));
    let (tensor, _) = letterbox_chw(&img, 640);
    assert_eq!(tensor.len(), 3 * 640 * 640);
}

#[test]
fn decode_and_map_tags_pipeline() {
    // 1×6×4 张量：anchor0 = class1 高分，anchor1 = class0 中分，anchor2/3 全 NaN 作废
    let mut data = vec![f32::NAN; 6 * 4];
    for (a, v) in [(0, 10.0f32), (1, 20.0)] {
        data[a] = v;
        data[4 + a] = v + 1.0;
        data[8 + a] = v + 2.0;
        data[12 + a] = v + 3.0;
    }
    // anchor0（坐标 10,11,12,13）：class0=0.1, class1=0.8 → class1 胜
    // anchor1（坐标 20,21,22,23）：class0=0.9, class1=0.2 → class0 胜
    let mut score = |class: usize, anchor: usize, v: f32| data[(4 + class) * 4 + anchor] = v;
    score(0, 0, 0.1);
    score(1, 0, 0.8);
    score(0, 1, 0.9);
    score(1, 1, 0.2);
    let dets = decode(&data, 4, 2);
    assert_eq!(dets.len(), 2);

    let names = vec!["chat_list".into(), "msg_input".into()];
    let regions = map_tags(&dets, &names, 0.45);
    assert_eq!(regions.len(), 2);
    assert!(regions.iter().any(|r| r.tag == "chat_list" && (r.confidence - 0.9).abs() < 1e-6));
    assert!(regions.iter().any(|r| r.tag == "msg_input" && (r.confidence - 0.8).abs() < 1e-6));
}

#[test]
fn rect_inverse_transform_clamped() {
    let geo = Letterbox::for_size(530, 774, 640);
    let r = rect_to_src(&geo, geo.pad_x, geo.pad_y, 100.0 * geo.scale, 50.0 * geo.scale, 530, 774);
    assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 100, 50));
}

// ---------------------------------------------------------------------------
// UT-LAY-05：类别表与 tags.json 不匹配 → 拒绝导入/加载
// ---------------------------------------------------------------------------

#[test]
fn ut_lay_05_class_mismatch_rejected() {
    let required: Vec<String> = ["chat_list", "chat_window", "chat_target", "msg_input"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // 覆盖全部 → 通过
    let full = ModelMeta {
        kind: "layout".into(),
        file: "m.onnx".into(),
        version: "1".into(),
        input_size: Some(640),
        classes: required.clone(),
        note: String::new(),
    };
    assert!(ModelStore::validate_classes(&full, &required).is_ok());

    // 缺 msg_input → 拒绝，错误信息含缺失项
    let partial = ModelMeta {
        classes: required[..3].to_vec(),
        ..full.clone()
    };
    let err = ModelStore::validate_classes(&partial, &required).unwrap_err();
    assert!(err.to_string().contains("msg_input"), "{err}");

    // 多余类别（超集）→ 允许（要求是「覆盖」而非「相等」）
    let superset = ModelMeta {
        classes: vec![
            "chat_list".into(),
            "chat_window".into(),
            "chat_target".into(),
            "msg_input".into(),
            "extra".into(),
        ],
        ..full
    };
    assert!(ModelStore::validate_classes(&superset, &required).is_ok());
}

// ---------------------------------------------------------------------------
// UT-LAY-04 / UT-LAY-06：Stage 级（用测试桩权重真实推理）
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::sync::Arc;

use dc_core::{ConfigValue, ImageLogSink, ModuleContext, RunId};
use dc_pipeline::contract::{LoopKind, PipelineContext};
use dc_pipeline::{LayoutStage, Module, Stage, WindowSnapshot};
use dc_sys::Rect;
use std::time::Instant;

/// 复制 fixtures 桩到临时 models/<name>/ 目录并构造 ModuleContext。
/// 返回 (ModuleContext, tmpdir)——tmpdir 生命周期必须覆盖 Stage 全部调用。
fn module_ctx(model: &str) -> (ModuleContext, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(model);
    std::fs::create_dir_all(&dir).unwrap();
    let stub = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let src = stub.join(format!("yolo-test-{model_kind}.onnx", model_kind = if model.contains("empty") { "empty" } else { "fixed" }));
    std::fs::copy(&src, dir.join("model.onnx")).unwrap();
    std::fs::write(
        dir.join("model.toml"),
        r#"kind = "layout"
file = "model.onnx"
version = "test-stub"
input_size = 320
classes = ["chat_list", "chat_window", "chat_target", "msg_input"]
"#,
    )
    .unwrap();

    let mut values = BTreeMap::new();
    values.insert("layout.model".to_string(), ConfigValue::Str(model.to_string()));
    values.insert("device.prefer_gpu".to_string(), ConfigValue::Bool(false)); // 测试环境强制 CPU（UT-LAY-06 路径）
    let mctx = ModuleContext {
        config: Arc::new(dc_core::ConfigSnapshot::new(values, 1)),
        models: Arc::new(ModelStore::new(tmp.path())),
        log_dir: tmp.path().to_path_buf(),
    };
    (mctx, tmp)
}

fn snapshot(w: u32, h: u32) -> WindowSnapshot {
    WindowSnapshot {
        image: image::RgbaImage::from_pixel(w, h, image::Rgba([120, 130, 140, 255])),
        window_rect: Rect::new(0, 0, w, h),
        dpi_scale: 1.0,
        captured_at: Instant::now(),
    }
}

fn run_ctx() -> PipelineContext {
    PipelineContext::new(
        RunId::from_raw("20260913-200000-000001"),
        LoopKind::Slow,
        ImageLogSink::new(std::path::PathBuf::from("unused"), RunId::from_raw("x"), false),
        Arc::new(dc_core::ConfigSnapshot::default()),
    )
}

/// UT-LAY-04：空检出（恒零输出桩）→ `regions: []` 而非 Err。
#[test]
fn ut_lay_04_empty_detection_returns_empty_not_err() {
    let (mctx, _tmp) = module_ctx("stub-empty");
    let mut stage = LayoutStage::new();
    stage.init(&mctx).expect("stub-empty init 应成功");

    let out = stage.process(snapshot(640, 480), &run_ctx()).expect("空检出不应 Err");
    assert!(out.regions.is_empty());
}

/// UT-LAY-06：GPU 不可用/被禁用时，CPU 回退路径可推理。
/// （prefer_gpu=false 直接走 CPU 链；固定框桩输出一个框，解码后应为 class0 的框。）
#[test]
fn ut_lay_06_cpu_fallback_infers() {
    let (mctx, _tmp) = module_ctx("stub-fixed");
    let mut stage = LayoutStage::new();
    stage.init(&mctx).expect("stub-fixed init 应成功");

    let out = stage.process(snapshot(640, 480), &run_ctx()).expect("CPU 推理不应 Err");
    assert_eq!(out.regions.len(), 1, "固定框桩应恰好检出 1 个: {:?}", out.regions);
    let r = &out.regions[0];
    assert_eq!(r.tag, "chat_list");
    assert!((r.confidence - 0.9).abs() < 1e-3, "score={}", r.confidence);
    // 桩在 320 letterbox 中心 (160,160) 放 80×80 框；640×480 原图 scale=0.5、pad=(0,40)：
    // 原图中心 = ((160-0)/0.5, (160-40)/0.5) = (320,240)，尺寸 80/0.5=160 → 左上 (240,160)
    assert_eq!((r.rect.x, r.rect.y), (240, 160), "rect={:?}", r.rect);
    assert_eq!((r.rect.w, r.rect.h), (160, 160), "rect={:?}", r.rect);
}

/// 模型缺失 → Fatal（错误矩阵）。
#[test]
fn missing_model_is_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut values = BTreeMap::new();
    values.insert("layout.model".to_string(), ConfigValue::Str("no-such".into()));
    let mctx = ModuleContext {
        config: Arc::new(dc_core::ConfigSnapshot::new(values, 1)),
        models: Arc::new(ModelStore::new(tmp.path())),
        log_dir: tmp.path().to_path_buf(),
    };
    let mut stage = LayoutStage::new();
    let err = stage.init(&mctx).unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
}
