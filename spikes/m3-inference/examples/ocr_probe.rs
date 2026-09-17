//! P0 探测（M4 计划）：用本地 PP-OCRv4 权重手动构造 RapidOcrConfig，
//! 验证 rapidocr-core 0.2.2 的 CPU 推理链路与参数正确性。
//!
//! ```bash
//! cargo run -p m3-inference --example ocr_probe -- <图片路径>
//! ```

use std::path::Path;

use rapidocr_core::config::{
    ClsConfig, DetConfig, DetInputLimits, InferenceOptions, LimitType, PipelineConfig,
    RapidOcrConfig, RecConfig,
};
use rapidocr_core::RapidOcr;

/// 构造 PP-OCRv4 中文三件套配置（models/ 下三个目录）。
///
/// 参数依据：
/// - det：PaddleOCR v4 官方 mean/std（0.485/0.456/0.406 ÷ 0.229/0.224/0.225），
///   其余 DBNet 参数与 rapidocr-core 的 PPOCRV6_DET_PARAMS 一致（thresh 0.3 / box 0.5 / unclip 1.6）；
/// - cls：PPOCRV4_CLS_PARAMS（[3,48,192] batch 6 thresh 0.9）；
/// - rec：[3,48,320] batch 6（crate 的 rec_with_assets 同值）。
pub fn ppocr_v4_zh(models_root: &Path) -> RapidOcrConfig {
    RapidOcrConfig {
        pipeline: PipelineConfig::full(),
        inference: InferenceOptions {
            intra_threads: 2,
            ..Default::default()
        },
        text_score: 0.5,
        min_side_len: 30,
        max_side_len: 2000,
        min_height: 30,
        width_height_ratio: 8.0,
        det: Some(DetConfig {
            model_path: models_root.join("ocr-det-ppocrv4/ch_PP-OCRv4_det_mobile.onnx"),
            limit_side_len: 736,
            limit_type: LimitType::Min,
            input_limits: DetInputLimits::default(),
            mean: [0.485, 0.456, 0.406],
            std: [0.229, 0.224, 0.225],
            thresh: 0.3,
            box_thresh: 0.5,
            max_candidates: 1000,
            unclip_ratio: 1.6,
            min_size: 3,
        }),
        cls: Some(ClsConfig {
            model_path: models_root.join("ocr-cls-ppocrv20/ch_ppocr_mobile_v2.0_cls_mobile.onnx"),
            image_shape: [3, 48, 192],
            batch_size: 6,
            thresh: 0.9,
            labels: vec!["0".into(), "180".into()],
        }),
        rec: Some(RecConfig {
            model_path: models_root.join("ocr-rec-ppocrv4/ch_PP-OCRv4_rec_mobile.onnx"),
            dict_path: models_root.join("ocr-rec-ppocrv4/ppocr_keys_v1.txt"),
            image_shape: [3, 48, 320],
            batch_size: 6,
        }),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let img_path = args
        .first()
        .map(String::as_str)
        .unwrap_or("resources/training_set/wechat_snapcaptures/Snipaste_2026-09-13_10-56-17.png");
    let models_root = std::path::PathBuf::from("resources/models");

    let img = image::open(img_path).expect("读图失败");
    println!("输入：{img_path}（{}×{}）", img.width(), img.height());

    let cfg = ppocr_v4_zh(&models_root);
    let t0 = std::time::Instant::now();
    let mut ocr = RapidOcr::new(cfg).expect("RapidOcr 初始化失败");
    let init_ms = t0.elapsed().as_millis();

    let rgb = img.to_rgb8();
    let t1 = std::time::Instant::now();
    let timed = ocr.run_image_timed(&rgb).expect("OCR 推理失败");
    let infer_ms = t1.elapsed().as_millis();

    println!(
        "\ninit {init_ms}ms    总推理 {infer_ms}ms（det {:.1} cls {:.1} rec {:.1}）",
        timed.timings.det_inference_ms,
        timed.timings.cls_inference_ms,
        timed.timings.rec_inference_ms
    );
    println!("识别 {} 行：", timed.output.lines.len());
    for line in &timed.output.lines {
        println!("  [{:.2}] {}", line.score, line.text);
    }
}
