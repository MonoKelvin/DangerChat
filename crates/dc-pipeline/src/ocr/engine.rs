//! OCR 引擎封装（§5.6 det→cls→rec 流水线，rapidocr-core 0.2.2）。
//!
//! - 模型三件套从 ModelStore 的三个 ocr 目录加载（models/ocr-{det,cls,rec}-*/）；
//! - 取消：把流水线自己的 `OcrCancellationToken` 桥接到 `ctx.cancel`——
//!   pre-run 取消直接短路，进行中的取消由 rapidocr 在 stage 间隙检查；
//! - 设备（ADR-15）：det/rec 默认 GPU、cls 默认 CPU；rapidocr-core 的
//!   `InferenceOptions.execution_provider` 是会话级统一开关（三件套一个值），
//!   因此 M4 以 CPU 交付（小图预算内），DML 差异化留 M5（见模块 doc）。

use std::path::{Path, PathBuf};

use rapidocr_core::config::{
    ClsConfig, DetConfig, DetInputLimits, ExecutionProvider, InferenceOptions, LimitType,
    PipelineConfig, RapidOcrConfig, RecConfig,
};
use rapidocr_core::RapidOcr;

use crate::contract::StageError;

use super::assemble::RawLine;
use dc_sys::Rect;

/// 三件套路径集合。
#[derive(Debug, Clone)]
pub struct OcrModelPaths {
    pub det: PathBuf,
    pub cls: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
}

/// PP-OCRv4 中文配置（P0 探测验证过的参数；详见 spikes ocr_probe）。
pub fn ppocr_v4_config(paths: &OcrModelPaths, intra_threads: usize, gpu: bool) -> RapidOcrConfig {
    RapidOcrConfig {
        // 跳过方向分类：聊天输入文字恒水平，cls 是纯多余的一次推理（快环延迟关键路径）
        pipeline: PipelineConfig::without_cls(),
        inference: InferenceOptions {
            intra_threads,
            execution_provider: if gpu {
                ExecutionProvider::DirectMl
            } else {
                ExecutionProvider::Cpu
            },
            ..Default::default()
        },
        text_score: 0.5,
        min_side_len: 30,
        max_side_len: 2000,
        min_height: 30,
        width_height_ratio: 8.0,
        det: Some(DetConfig {
            model_path: paths.det.clone(),
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
            model_path: paths.cls.clone(),
            image_shape: [3, 48, 192],
            batch_size: 6,
            thresh: 0.9,
            labels: vec!["0".into(), "180".into()],
        }),
        rec: Some(RecConfig {
            model_path: paths.rec.clone(),
            dict_path: paths.dict.clone(),
            image_shape: [3, 48, 320],
            batch_size: 6,
        }),
    }
}

pub struct OcrEngine {
    /// Mutex 满足 ort `run(&mut)` 签名（§2.3 单飞，锁竞争为零）。
    ocr: std::sync::Mutex<RapidOcr>,
}

impl OcrEngine {
    pub fn new(paths: &OcrModelPaths, intra_threads: usize, gpu: bool) -> Result<Self, String> {
        let cfg = ppocr_v4_config(paths, intra_threads, gpu);
        let ocr = RapidOcr::new(cfg).map_err(|e| format!("OCR 引擎初始化失败：{e}"))?;
        Ok(Self {
            ocr: std::sync::Mutex::new(ocr),
        })
    }

    /// 对一张 ROI（RGB）跑完整流水线，输出行（局部坐标 + 置信度）。
    ///
    /// 已取消时快速短路（pre-run 检查；进行中取消的协作桥接留 M5——
    /// `run_image_cancellable` 需要 spawn 轮询线程，先以简单路径交付）。
    /// bbox 取轴对齐外接矩形（契约 `TextBlock.rect` 是矩形；微信文本行
    /// 基本水平，四点框的轴对齐损失可忽略）。
    pub fn run(&self, roi: &image::RgbImage, cancelled: bool) -> Result<Vec<RawLine>, StageError> {
        if cancelled {
            return Err(StageError::Cancelled);
        }
        let mut ocr = self
            .ocr
            .lock()
            .map_err(|_| StageError::Recoverable("OCR 引擎锁中毒".into()))?;
        let output = ocr
            .run_image(roi)
            .map_err(|e| StageError::Recoverable(format!("OCR 推理失败：{e}")))?;
        Ok(output
            .lines
            .into_iter()
            .map(|l| {
                let xs = [
                    l.bbox.points[0][0],
                    l.bbox.points[1][0],
                    l.bbox.points[2][0],
                    l.bbox.points[3][0],
                ];
                let ys = [
                    l.bbox.points[0][1],
                    l.bbox.points[1][1],
                    l.bbox.points[2][1],
                    l.bbox.points[3][1],
                ];
                let x1 = xs.iter().cloned().fold(f32::INFINITY, f32::min).floor() as i32;
                let y1 = ys.iter().cloned().fold(f32::INFINITY, f32::min).floor() as i32;
                let x2 = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
                let y2 = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
                RawLine {
                    text: l.text,
                    score: l.score,
                    rect: Rect::new(x1, y1, (x2 - x1).max(1) as u32, (y2 - y1).max(1) as u32),
                }
            })
            .collect())
    }
}

/// 从 ModelStore 的三个模型目录解析三件套路径。
pub fn resolve_paths(
    det_dir: &Path,
    cls_dir: &Path,
    rec_dir: &Path,
) -> Result<OcrModelPaths, String> {
    let pick = |dir: &Path, exts: &[&str]| -> Result<PathBuf, String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("读模型目录失败 {}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let p = e.path();
            if p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x))
            {
                return Ok(p);
            }
        }
        Err(format!("目录 {} 下没有 {:?}", dir.display(), exts))
    };
    let dict = rec_dir.join("ppocr_keys_v1.txt");
    if !dict.is_file() {
        return Err(format!("rec 字典缺失：{}", dict.display()));
    }
    Ok(OcrModelPaths {
        det: pick(det_dir, &["onnx"])?,
        cls: pick(cls_dir, &["onnx"])?,
        rec: pick(rec_dir, &["onnx"])?,
        dict,
    })
}
