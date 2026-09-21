//! 文字识别模块（设计文档 §5.6）。`Stage<(WindowSnapshot, RegionLayout), OcrResult>`。
//!
//! 算法（§5.6）：
//! - 快环（ctx.loop_kind == Fast）：仅 msg_input ROI（layout 需已 `input_only()`）；
//! - 慢环：chat_target + chat_window + msg_input 三 ROI；
//!   `chat_context` 取 chat_window 区域最近 N 行拼接，供 L2 语义判定消歧（§5.7-context）
//! - ROI 高度 < 32px 时 2× Lanczos 超分（`ocr.upscale`）；
//! - `draft_text` 行拼接 + 噪声过滤（assemble 纯函数）；`chat_target` 取置信度最高行；
//! - 错误矩阵：ROI 空/过小 → 对应字段空值（**非 Err**）；推理异常 → Recoverable。
//!
//! 设备（ADR-15）：det/rec 默认 GPU、cls 默认 CPU；但 rapidocr-core 的 EP 是
//! 会话级统一开关（三件套一个值），M4 以 CPU 交付（P0 实测：小 ROI 在快环
//! 预算内），差异化 EP 留 M5 评估（届时可 fork 会话或换逐模型实例）。

pub mod assemble;
pub mod engine;

use std::time::Instant;

use dc_core::{
    ConfigField, ConfigType, ConfigValue, MetricsRecorder, ModuleContext, ModuleError,
    ModuleMetrics,
};

use crate::contract::{
    Module, OcrResult, PipelineContext, RegionLayout, Stage, StageError, WindowSnapshot,
    TAG_CHAT_TARGET, TAG_CHAT_WINDOW, TAG_MSG_INPUT,
};

/// ROI 高度低于此值时 2× 超分（PaddleOCR rec 输入高 48，小图直接识别质量差）。
const UPSCALE_BELOW: u32 = 32;

/// 默认噪声词表：整行 OCR 命中即过滤（§5.6）。
///
/// 覆盖发送按钮、语音输入占位，以及聊天 UI 边角切出的孤立碎片：
/// - 符号/标点孤立行：`)` `(` `（` `%`
/// - 孤立拉丁字母：`M` `Y` `R`
/// - 孤立 CJK 及其带括号伪影：`白` `白(` `白（`
/// - 孤立问号：`?`
///
/// 这些碎片由噪声词表负责，**而非长度阈值**——长度阈值会误删「sb」「傻逼」等
/// 短敏感词（见 `assemble::filter_lines`）。本表可通过设置 `ocr.noise_words` 覆盖。
const DEFAULT_NOISE_WORDS: &[&str] = &[
    "发送",
    "按住鼠标 语音输入文字",
    ")",
    "(",
    "（",
    "%",
    "M",
    "Y",
    "R",
    "?",
    "白",
    "白(",
    "白（",
];

pub struct OcrStage {
    engine: Option<engine::OcrEngine>,
    noise_words: Vec<String>,
    min_conf: f64,
    upscale: bool,
    metrics: MetricsRecorder,
}

/// 聊天窗口上下文摘要最大行数（BGE 嵌入输入预算 ≈ 64 token）。
const CONTEXT_MAX_LINES: usize = 5;

impl OcrStage {
    pub fn new() -> Self {
        Self {
            engine: None,
            noise_words: vec!["发送".into(), "按住鼠标 语音输入文字".into()],
            min_conf: 0.5,
            upscale: true,
            metrics: MetricsRecorder::default(),
        }
    }

    /// ROI 裁剪（layout 窗口相对坐标 → 图象局部像素）+ 可选 2× 超分，转 RGB。
    ///
    /// `rect` 来自 layout，为窗口相对坐标（距 window_rect 左上）。
    /// `snapshot.origin` 告诉图象在屏幕上的起点：
    /// - 全窗截图：origin == window_rect，offset = (0, 0)
    /// - ROI 裁剪：origin 为 ROI 屏幕矩形，offset = -(ROI.left - window.left, ROI.top - window.top)
    fn crop_roi(
        snapshot: &WindowSnapshot,
        rect: &dc_sys::Rect,
        upscale: bool,
    ) -> Option<image::RgbImage> {
        let (w, h) = (snapshot.image.width(), snapshot.image.height());
        // 窗口相对 → 图象局部：减去 origin 与 window_rect 的差值
        let ox = snapshot.origin.x - snapshot.window_rect.x;
        let oy = snapshot.origin.y - snapshot.window_rect.y;
        let x = (rect.x - ox).clamp(0, w as i32).max(0) as u32;
        let y = (rect.y - oy).clamp(0, h as i32).max(0) as u32;
        let rw = rect.w.min(w.saturating_sub(x));
        let rh = rect.h.min(h.saturating_sub(y));
        if rw < 4 || rh < 4 {
            return None; // 空/过小 ROI → 字段空值（非 Err）
        }
        let crop = image::imageops::crop_imm(&snapshot.image, x, y, rw, rh).to_image();
        let rgb = image::DynamicImage::ImageRgba8(crop).to_rgb8();
        if upscale && rh < UPSCALE_BELOW {
            Some(
                image::DynamicImage::ImageRgb8(rgb)
                    .resize_exact(rw * 2, rh * 2, image::imageops::FilterType::Lanczos3)
                    .to_rgb8(),
            )
        } else {
            Some(rgb)
        }
    }

    fn recognize(
        &self,
        input: (WindowSnapshot, RegionLayout),
        ctx: &PipelineContext,
    ) -> Result<OcrResult, StageError> {
        let (snapshot, layout) = &input;
        let Some(engine) = self.engine.as_ref() else {
            return Err(StageError::Fatal("OCR 引擎未初始化".into()));
        };

        // 快环仅输入框；慢环双 ROI + chat_window 上下文（§5.6/§5.7-context）
        let tags: &[&str] = match ctx.loop_kind {
            crate::contract::LoopKind::Fast => &[TAG_MSG_INPUT],
            _ => &[TAG_CHAT_TARGET, TAG_CHAT_WINDOW, TAG_MSG_INPUT],
        };

        let mut chat_target: Option<String> = None;
        let mut chat_context: Option<String> = None;
        let mut draft_blocks: Vec<assemble::RawLine> = Vec::new();

        for tag in tags {
            ctx.cancel.check()?;
            let Some(region) = layout.regions.iter().find(|r| r.tag == *tag) else {
                continue; // 该 tag 无检出 → 字段空值（非 Err）
            };
            let Some(roi) = Self::crop_roi(snapshot, &region.rect, self.upscale) else {
                continue;
            };
            let lines = engine.run(&roi, ctx.cancel.is_cancelled())?;

            match *tag {
                TAG_MSG_INPUT => draft_blocks = lines,
                TAG_CHAT_TARGET => {
                    chat_target = assemble::pick_chat_target(&lines, self.min_conf as f32)
                }
                TAG_CHAT_WINDOW => {
                    // 噪声词过滤（如发送按钮）后提取最近 N 行上下文
                    let filtered =
                        assemble::filter_lines(lines, &self.noise_words, self.min_conf as f32);
                    chat_context = assemble::extract_chat_context(
                        &filtered,
                        self.min_conf as f32,
                        CONTEXT_MAX_LINES,
                    )
                }
                _ => {}
            }
        }

        // 03_ocr 可视化已移除（按用户要求只保留 01_window + 02_layout）

        let mut result = assemble::assemble(draft_blocks, &self.noise_words, self.min_conf as f32);
        result.chat_target = chat_target;
        result.chat_context = chat_context;
        Ok(result)
    }
}

impl Default for OcrStage {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for OcrStage {
    fn id(&self) -> &'static str {
        "ocr"
    }

    fn config_schema(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "ocr.upscale".into(),
                ty: ConfigType::Bool,
                default: ConfigValue::Bool(true),
                label: "小图超分".into(),
                help: "输入框高度 <32px 时 2× 放大再识别".into(),
                group: "模型与设备".into(),
                owner: "ocr".into(),
            },
            ConfigField {
                key: "ocr.min_conf".into(),
                ty: ConfigType::Float { min: 0.1, max: 0.9 },
                default: ConfigValue::Float(0.5),
                label: "最低置信度".into(),
                help: "低于此值的文本行被丢弃".into(),
                group: "模型与设备".into(),
                owner: "ocr".into(),
            },
            ConfigField {
                key: "ocr.noise_words".into(),
                ty: ConfigType::StrList,
                default: ConfigValue::StrList(
                    DEFAULT_NOISE_WORDS.iter().map(|s| s.to_string()).collect(),
                ),
                label: "噪声词表".into(),
                help: "整行命中即过滤（发送按钮一类）".into(),
                group: "模型与设备".into(),
                owner: "ocr".into(),
            },
        ]
    }

    fn init(&mut self, mctx: &ModuleContext) -> Result<(), ModuleError> {
        // OCR 三件套是内置模型（随安装包 resources/models/）：按名解析，
        // 用户层存在优先（允许用户覆盖），否则用内置层。
        let resolve = |name: &str| {
            mctx.models
                .resolve_dir(name)
                .ok_or_else(|| ModuleError::Fatal(format!("OCR 模型目录不存在：{name}")))
        };
        let det = resolve("ocr-det-ppocrv4")?;
        let cls = resolve("ocr-cls-ppocrv20")?;
        let rec = resolve("ocr-rec-ppocrv4")?;
        let paths = engine::resolve_paths(&det, &cls, &rec).map_err(ModuleError::Fatal)?;
        let intra = mctx.config.i64_or("device.intra_threads", 2).clamp(1, 4) as usize;
        let gpu = mctx.config.str_or("ocr.device", "direct-ml") == "direct-ml";
        let engine = engine::OcrEngine::new(&paths, intra, gpu).map_err(ModuleError::Fatal)?;
        let default_noise: Vec<String> =
            DEFAULT_NOISE_WORDS.iter().map(|s| s.to_string()).collect();
        self.noise_words = mctx.config.str_list_or("ocr.noise_words", &default_noise);
        self.min_conf = mctx.config.f64_or("ocr.min_conf", 0.5);
        self.upscale = mctx.config.bool_or("ocr.upscale", true);
        self.engine = Some(engine);
        tracing::info!(
            "OCR 引擎已加载（PP-OCRv4 mobile，{}）",
            if gpu { "DirectML GPU" } else { "CPU" }
        );
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ModuleError> {
        self.engine = None;
        Ok(())
    }

    fn metrics(&self) -> ModuleMetrics {
        self.metrics.snapshot()
    }
}

impl Stage for OcrStage {
    type Input = (WindowSnapshot, RegionLayout);
    type Output = OcrResult;

    fn process(
        &self,
        input: Self::Input,
        ctx: &PipelineContext,
    ) -> Result<Self::Output, StageError> {
        let started = Instant::now();
        let result = self.recognize(input, ctx);
        self.metrics.record(started.elapsed());
        if result.is_err() {
            self.metrics.record_failure();
        }
        result
    }
}
