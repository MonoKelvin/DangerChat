//! 区域划分模块（设计文档 §5.5）。`Stage<WindowSnapshot, RegionLayout>`。
//!
//! 错误矩阵（§5.5）：
//! - 模型文件缺失 / model.toml 非法 / 类别不覆盖 → **Fatal**（前端引导导入）；
//! - 单次推理异常 → **Recoverable**（本轮跳过，fail-open）；
//! - GPU EP 初始化失败 → 自动 CPU + warn（session.rs 内处理）。

pub mod postprocess;
pub mod preprocess;
pub mod session;

use std::time::Instant;

use dc_core::{
    ConfigField, ConfigType, ConfigValue, MetricsRecorder, ModuleContext, ModuleError,
    ModuleMetrics,
};

use crate::contract::{
    Module, PipelineContext, Region, RegionLayout, Stage, StageError, WindowSnapshot,
};

/// 画框可视化（02_layout 图片日志）：在快照副本上描出各区域。
fn draw_regions(base: &WindowSnapshot, regions: &[Region]) -> image::RgbaImage {
    let mut img = base.image.clone();
    for r in regions {
        let color = match r.tag.as_str() {
            crate::contract::TAG_CHAT_LIST => image::Rgba([76, 154, 255, 255]),
            crate::contract::TAG_CHAT_WINDOW => image::Rgba([54, 179, 126, 255]),
            crate::contract::TAG_CHAT_TARGET => image::Rgba([255, 171, 0, 255]),
            _ => image::Rgba([255, 86, 48, 255]),
        };
        let (x, y) = (r.rect.x.max(0), r.rect.y.max(0));
        let (w, h) = (r.rect.w as i32, r.rect.h as i32);
        for dx in 0..w {
            for dy in [0, h - 1] {
                if let Some(p) = img.get_pixel_mut_checked((x + dx) as u32, (y + dy) as u32) {
                    *p = color;
                }
            }
        }
        for dy in 0..h {
            for dx in [0, w - 1] {
                if let Some(p) = img.get_pixel_mut_checked((x + dx) as u32, (y + dy) as u32) {
                    *p = color;
                }
            }
        }
    }
    img
}

pub struct LayoutStage {
    session: Option<session::InferSession>,
    /// model.toml 的类别表（index → tag 名）。
    class_names: Vec<String>,
    metrics: MetricsRecorder,
    /// init 之后固定（进程生命周期内不重载；挂起/唤醒的模型装卸载是 M5 guard 的职责）。
    conf_threshold: f64,
    nms_iou: f64,
}

impl LayoutStage {
    pub fn new() -> Self {
        Self {
            session: None,
            class_names: Vec::new(),
            metrics: MetricsRecorder::default(),
            conf_threshold: 0.45,
            nms_iou: 0.5,
        }
    }

    fn infer(
        &self,
        snapshot: &WindowSnapshot,
        ctx: &PipelineContext,
    ) -> Result<RegionLayout, StageError> {
        ctx.cancel.check()?;
        let Some(sess) = &self.session else {
            // 模型未加载（init 失败/未导入）：窗口底部区域兜底为输入区。
            // 聊天输入框恒在窗口底部——只 OCR 底部 30% 区域，比整窗快 3~5 倍，
            // 快环缓存后打字停顿只需跑小区域，判定延迟进入秒级。
            // 不 Fatal——否则 Guard 整体停用，规则拦截也一并失效。
            tracing::warn!(
                "layout 模型未加载，输入区按窗口底部 30% 兜底（设置→模型与设备 导入后恢复精确定位）"
            );
            let (w, h) = (snapshot.image.width(), snapshot.image.height());
            let band = (h * 3 / 10).max(80);
            return Ok(RegionLayout {
                regions: vec![Region {
                    tag: crate::contract::TAG_MSG_INPUT.into(),
                    rect: dc_sys::Rect::new(0, (h - band) as i32, w, band),
                    confidence: 0.0,
                }],
                layout_epoch: 0,
                inferred_at: Instant::now(),
            });
        };

        // 1) letterbox：RGBA → 1×3×side×side CHW
        let side = sess.input_side;
        let (chw, geo) = preprocess::letterbox_chw(&snapshot.image, side);
        ctx.cancel.check()?;

        // 2) 推理
        let (flat, anchors, classes) = sess.run(chw, side)?;
        ctx.cancel.check()?;

        // 3) 解码 → NMS → TagMapper（每 tag 最高置信度 1 个）
        let dets = postprocess::decode(&flat, anchors, classes);
        let kept = postprocess::nms(&dets, self.nms_iou as f32);
        let regions: Vec<Region> = kept
            .iter()
            .filter(|d| d.score >= self.conf_threshold as f32)
            .filter_map(|d| {
                let tag = self.class_names.get(d.class)?.clone();
                let rect = preprocess::rect_to_src(
                    &geo,
                    d.x - d.w / 2.0,
                    d.y - d.h / 2.0,
                    d.w,
                    d.h,
                    snapshot.image.width(),
                    snapshot.image.height(),
                );
                Some(Region {
                    tag,
                    rect,
                    confidence: d.score,
                })
            })
            .collect();
        // map_tags 的「每 tag 取最高」语义在此实现（kept 已按分数降序，先见者胜）
        let mut best: Vec<Region> = Vec::new();
        for r in regions {
            match best.iter_mut().find(|b| b.tag == r.tag) {
                Some(prev) => {
                    if r.confidence > prev.confidence {
                        *prev = r;
                    }
                }
                None => best.push(r),
            }
        }

        // 4) 图片日志（名称带 `<run_id>/` 前缀，见 capture.rs 同处说明）
        let name = format!("{}/02_layout", ctx.run_id.as_str());
        if let Err(err) = ctx.image_log.save(&name, &draw_regions(snapshot, &best)) {
            tracing::warn!(error = %err, "layout 可视化写入图片日志失败");
        }

        Ok(RegionLayout {
            regions: best,
            layout_epoch: 0,
            inferred_at: Instant::now(),
        })
    }
}

impl Default for LayoutStage {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for LayoutStage {
    fn id(&self) -> &'static str {
        "layout"
    }

    fn config_schema(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "layout.model".into(),
                ty: ConfigType::Text { max_len: 64 },
                default: ConfigValue::Str("dc-layout-wechat".into()),
                label: "区域模型".into(),
                help: "models/ 下的模型目录名".into(),
                group: "模型与设备".into(),
                owner: "layout".into(),
            },
            ConfigField {
                key: "layout.conf_threshold".into(),
                ty: ConfigType::Float {
                    min: 0.05,
                    max: 0.95,
                },
                default: ConfigValue::Float(0.45),
                label: "置信度阈值".into(),
                help: "低于此值的检出被丢弃".into(),
                group: "模型与设备".into(),
                owner: "layout".into(),
            },
            ConfigField {
                key: "layout.nms_iou".into(),
                ty: ConfigType::Float { min: 0.1, max: 0.9 },
                default: ConfigValue::Float(0.5),
                label: "NMS IoU 阈值".into(),
                help: "同类别重叠超过此值的低分框被抑制".into(),
                group: "模型与设备".into(),
                owner: "layout".into(),
            },
        ]
    }

    /// 经 ModelStore 加载模型并校验类别覆盖（UT-LAY-05：不覆盖 → Fatal）。
    fn init(&mut self, mctx: &ModuleContext) -> Result<(), ModuleError> {
        let model_name = mctx.config.str_or("layout.model", "dc-layout-wechat");
        let info = mctx.models.get(model_name).ok_or_else(|| {
            ModuleError::Fatal(format!("layout 模型不存在：{model_name}（请在设置中导入）"))
        })?;
        if info.kind() != Some(dc_core::ModelKind::Layout) {
            return Err(ModuleError::Fatal(format!(
                "模型 {model_name} 的 kind 不是 layout"
            )));
        }

        // 类别覆盖校验（§5.5 模型管理）
        let required = [
            crate::contract::TAG_CHAT_LIST,
            crate::contract::TAG_CHAT_WINDOW,
            crate::contract::TAG_CHAT_TARGET,
            crate::contract::TAG_MSG_INPUT,
        ];
        let required: Vec<String> = required.iter().map(|s| s.to_string()).collect();
        dc_core::ModelStore::validate_classes(&info.meta, &required)
            .map_err(|e| ModuleError::Fatal(format!("类别校验失败：{e}")))?;

        // 设备解析（ADR-15：layout 默认 GPU；prefer_gpu=false 全局强制 CPU）
        let prefer_gpu = mctx.config.bool_or("device.prefer_gpu", true);
        let device = session::resolve_device(session::LAYOUT_DEFAULT_PREF, prefer_gpu);

        let intra = mctx.config.i64_or("device.intra_threads", 2).clamp(1, 4) as usize;
        let sess = session::InferSession::load(&info.weight_path(), device, intra)
            .map_err(ModuleError::Fatal)?;
        if sess.on_gpu {
            tracing::info!(model = %model_name, "layout 模型已加载（DirectML）");
        } else {
            tracing::info!(model = %model_name, "layout 模型已加载（CPU）");
        }

        self.class_names = info.meta.classes.clone();
        self.conf_threshold = mctx.config.f64_or("layout.conf_threshold", 0.45);
        self.nms_iou = mctx.config.f64_or("layout.nms_iou", 0.5);
        self.session = Some(sess);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ModuleError> {
        self.session = None;
        Ok(())
    }

    fn metrics(&self) -> ModuleMetrics {
        self.metrics.snapshot()
    }
}

impl Stage for LayoutStage {
    type Input = WindowSnapshot;
    type Output = RegionLayout;

    fn process(
        &self,
        input: Self::Input,
        ctx: &PipelineContext,
    ) -> Result<Self::Output, StageError> {
        let started = Instant::now();
        let result = self.infer(&input, ctx);
        self.metrics.record(started.elapsed());
        if result.is_err() {
            self.metrics.record_failure();
        }
        result
    }
}
