//! 截图模块（设计文档 §5.4）。`Stage<CaptureRequest, WindowSnapshot>`。
//!
//! 实现要点：
//! - 只通过屏幕 DC 拷贝**目标窗口矩形**（ADR-12），不做任何窗口定向捕获（§4.1）；
//! - `window_rect` 已由 dc-sys 完成 DPI 换算，这里**禁止再乘缩放比**（否则 1.25/1.5 缩放下必然错位）；
//! - 目标最小化或不在前台 → `Recoverable`（本轮跳过，按键按 fail-open 放行，FR-CAP-03）。

use std::sync::Arc;
use std::time::Instant;

use dc_core::{ConfigField, MetricsRecorder, ModuleContext, ModuleError, ModuleMetrics};
use dc_sys::{Hwnd, Rect, SysApi};

use crate::contract::{LoopKind, Module, PipelineContext, Stage, StageError, WindowSnapshot};

/// 截图请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRequest {
    pub hwnd: Hwnd,
    /// 可选 ROI 提示：仅截取窗口内相对矩形（屏幕坐标）。用于快环/心跳仅需输入框区域时，
    /// 避免拷贝全窗口位图（§2.3 耗时约束）。为 None 时截全窗口。
    pub roi: Option<Rect>,
}

pub struct CaptureStage {
    sys: Arc<dyn SysApi>,
    metrics: MetricsRecorder,
}

impl CaptureStage {
    pub fn new(sys: Arc<dyn SysApi>) -> Self {
        Self {
            sys,
            metrics: MetricsRecorder::default(),
        }
    }

    fn capture(
        &self,
        req: CaptureRequest,
        ctx: &PipelineContext,
    ) -> Result<WindowSnapshot, StageError> {
        ctx.cancel.check()?;

        // 1) 窗口可用性（FR-CAP-03：最小化/非前台 → 明确不可用，而不是返回错误图片）
        if self.sys.is_window_minimized(req.hwnd) {
            return Err(StageError::Recoverable("窗口已最小化".to_string()));
        }
        match self.sys.foreground() {
            Some(fg) if fg.hwnd == req.hwnd => {}
            other => {
                let got = other.map(|f| f.process_name).unwrap_or_default();
                return Err(StageError::Recoverable(format!(
                    "目标窗口不在前台（当前前台：{got}）"
                )));
            }
        }

        // 2) 矩形：已是物理像素，不再做二次 DPI 换算
        let (window_rect, dpi_scale) = self
            .sys
            .window_rect(req.hwnd)
            .map_err(|e| StageError::Recoverable(format!("取窗口矩形失败：{e}")))?;
        if window_rect.is_empty() {
            return Err(StageError::Recoverable("窗口矩形为空".to_string()));
        }

        // 3) 屏幕像素直拷
        //    - 快环/心跳：仅需 msg_input ROI，避免拷贝巨幅全窗口（§2.3 耗时约束）。
        //    - 慢环：拷贝全窗口，以便 layout 阶段推断区域。
        let capture_rect = req.roi.unwrap_or(window_rect);
        let image = self
            .sys
            .capture_region(capture_rect)
            .map_err(|e| StageError::Recoverable(format!("截图失败：{e}")))?;

        ctx.cancel.check()?;

        // 4) 图片日志（默认 Noop；落盘失败不阻断本轮，debug 图片不是关键路径）
        //    快环图像仅含输入框裁剪，不记录全窗口 debug 图，降低 IO 压（ADR-12）。
        if ctx.loop_kind != LoopKind::Fast && ctx.loop_kind != LoopKind::Heartbeat {
            let name = format!("{}/01_window", ctx.run_id.as_str());
            if let Err(err) = ctx.image_log.save(&name, &image) {
                tracing::warn!(error = %err, "窗口截图写入图片日志失败");
            }
        }

        Ok(WindowSnapshot {
            image,
            window_rect,
            origin: capture_rect,
            dpi_scale,
            captured_at: Instant::now(),
        })
    }
}

impl Module for CaptureStage {
    fn id(&self) -> &'static str {
        "capture"
    }

    /// capture 无语义配置项：图片日志开关归 dc-core（`privacy.no_image_logs`）。
    fn config_schema(&self) -> Vec<ConfigField> {
        Vec::new()
    }

    fn init(&mut self, _ctx: &ModuleContext) -> Result<(), ModuleError> {
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ModuleError> {
        Ok(())
    }

    fn metrics(&self) -> ModuleMetrics {
        self.metrics.snapshot()
    }
}

impl Stage for CaptureStage {
    type Input = CaptureRequest;
    type Output = WindowSnapshot;

    fn process(
        &self,
        input: Self::Input,
        ctx: &PipelineContext,
    ) -> Result<Self::Output, StageError> {
        let started = Instant::now();
        let result = self.capture(input, ctx);
        self.metrics.record(started.elapsed());
        if result.is_err() {
            self.metrics.record_failure();
        }
        result
    }
}
