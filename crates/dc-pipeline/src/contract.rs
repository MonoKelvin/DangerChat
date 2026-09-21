//! 模块统一契约（设计文档 §4）。
//!
//! - [`Stage`]：流水线阶段，`纯输入 → 输出`，错误即降级。
//! - [`PipelineContext`]：本轮运行的注入依赖（run id / 循环类型 / 取消令牌 / 图片日志 / 设备偏好 / 配置快照）。
//! - DTO：跨模块**唯一**数据格式，模块之间只传 DTO，不传实现。
//!
//! ## 与文档的一处偏差（已确认）
//!
//! `Module` 生命周期契约定义在 `dc-core`（因为 §5.1 把 `PluginRegistry` 放在 dc-core，而依赖方向是
//! dc-pipeline → dc-core，不能反向）。本模块把它重导出为 [`Module`]，并在此定义流水线专属的 [`Stage`]。
//!
//! ## 序列化说明
//!
//! DTO 的 serde / ts-rs 双导出在 M6（dc-bridge 落地时）补齐：届时才存在前端类型消费方。
//! 含 `Instant` / `RgbaImage` 的 DTO（`WindowSnapshot` / `RegionLayout` / `Verdict`）需要显式适配器，
//! 现在写半成品序列化只会制造假接口。

use std::sync::Arc;
use std::time::Instant;

use dc_core::{ConfigSnapshot, ImageLogSink, RunId};
use dc_sys::Rect;
use image::RgbaImage;

pub use dc_core::Module;

/// 流水线阶段。实现者必须是 `Send + Sync`（在 pipeline-worker 单线程执行、被多处读取指标）。
pub trait Stage: Module {
    type Input: Send;
    type Output: Send;

    /// 约定（§4）：
    /// - 不读全局状态；运行期数据全部经 [`PipelineContext`] 注入；
    /// - 尊重 `ctx.cancel`：被取消时尽快返回 `Err(StageError::Cancelled)`；
    /// - 除 `ctx.image_log` 之外不产生副作用。
    fn process(
        &self,
        input: Self::Input,
        ctx: &PipelineContext,
    ) -> Result<Self::Output, StageError>;
}

/// 阶段错误。`Recoverable` → 本轮跳过（fail-open 的关键路径）；`Fatal` → 模块停用 + 前端告警。
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("recoverable: {0}")]
    Recoverable(String),
    #[error("fatal: {0}")]
    Fatal(String),
    #[error("cancelled")]
    Cancelled,
}

impl StageError {
    /// 是否属于「本轮跳过、下次还有机会」的软失败。
    pub fn is_recoverable(&self) -> bool {
        matches!(self, StageError::Recoverable(_))
    }
}

/// 本轮属于快环还是慢环（心跳不走 Stage，仅 capture + pHash，§5.8）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    Fast,
    Slow,
    Heartbeat,
}

/// 推理设备偏好（`device.prefer_gpu` + 用户强制 CPU）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevicePref {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

/// 协作式取消令牌（不引 tokio_util，语义等价：置位后所有 `check` 立即返回 Cancelled）。
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Stage 内每步之间调用；已取消则中断本轮（§2.3「可取消」）。
    pub fn check(&self) -> Result<(), StageError> {
        if self.is_cancelled() {
            Err(StageError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// 一次流水线运行的上下文。
pub struct PipelineContext {
    /// 本轮唯一 ID，贯穿图片日志目录名。
    pub run_id: RunId,
    pub loop_kind: LoopKind,
    pub cancel: CancellationToken,
    /// debug 图片落盘句柄（隐私模式下为 Noop，调用侧无需分支）。
    pub image_log: ImageLogSink,
    pub device: DevicePref,
    /// 本轮开始时的配置快照：**运行中不被修改**（§4）。
    pub config: Arc<ConfigSnapshot>,
}

impl PipelineContext {
    pub fn new(
        run_id: RunId,
        loop_kind: LoopKind,
        image_log: ImageLogSink,
        config: Arc<ConfigSnapshot>,
    ) -> Self {
        Self {
            run_id,
            loop_kind,
            cancel: CancellationToken::new(),
            image_log,
            device: DevicePref::Auto,
            config,
        }
    }

    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_device(mut self, device: DevicePref) -> Self {
        self.device = device;
        self
    }
}

// ---------------------------------------------------------------------------
// 核心 DTO（§4.1）
// ---------------------------------------------------------------------------

/// capture 输出：裁剪后的目标窗口图 + 几何信息。
pub struct WindowSnapshot {
    pub image: RgbaImage,
    /// 屏幕坐标系（物理像素）：完整窗口矩形。layout 区域 rect 的坐标原点即为此矩形左上。
    pub window_rect: Rect,
    /// 屏幕坐标系（物理像素）：当前图象的左上角所在屏幕坐标。
    /// 全窗截图时 == window_rect；ROI 裁剪时为该 ROI 的屏幕矩形。
    /// 用于将 layout 区域 rect（窗口相对）映射到图象像素坐标（§2.3 快环 ROI 裁剪）。
    pub origin: Rect,
    pub dpi_scale: f32,
    pub captured_at: Instant,
}

impl Clone for WindowSnapshot {
    fn clone(&self) -> Self {
        Self {
            image: self.image.clone(),
            window_rect: self.window_rect,
            origin: self.origin,
            dpi_scale: self.dpi_scale,
            captured_at: self.captured_at,
        }
    }
}

impl std::fmt::Debug for WindowSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowSnapshot")
            .field("size", &(self.image.width(), self.image.height()))
            .field("window_rect", &self.window_rect)
            .field("origin", &self.origin)
            .field("dpi_scale", &self.dpi_scale)
            .finish()
    }
}

/// 一个被检出的区域。
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// 标签名，取值来自 `tags.json`（`chat_list` / `chat_window` / `chat_target` / `msg_input`）。
    pub tag: String,
    /// 相对窗口图左上角的坐标（原图像素系）。
    pub rect: Rect,
    pub confidence: f32,
}

/// layout 输出；同时作为慢环缓存（§2.3）。
#[derive(Debug, Clone, PartialEq)]
pub struct RegionLayout {
    /// 每 tag 取置信度最高 0..1 个（v1.0）。
    pub regions: Vec<Region>,
    /// 布局纪元，窗口变化即失效。
    pub layout_epoch: u64,
    pub inferred_at: Instant,
}

impl RegionLayout {
    pub fn rect_of(&self, tag: &str) -> Option<Rect> {
        self.regions.iter().find(|r| r.tag == tag).map(|r| r.rect)
    }

    /// 仅保留输入框区域的布局（快环复用：只重 OCR 输入框，§2.3）。
    pub fn input_only(&self) -> RegionLayout {
        RegionLayout {
            regions: self
                .regions
                .iter()
                .filter(|r| r.tag == TAG_MSG_INPUT)
                .cloned()
                .collect(),
            layout_epoch: self.layout_epoch,
            inferred_at: self.inferred_at,
        }
    }
}

/// 内置标签名（§5.5 / tags.json）。
pub const TAG_CHAT_LIST: &str = "chat_list";
pub const TAG_CHAT_WINDOW: &str = "chat_window";
pub const TAG_CHAT_TARGET: &str = "chat_target";
pub const TAG_MSG_INPUT: &str = "msg_input";

/// 一个文本块（OCR 输出）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    pub text: String,
    pub rect: Rect,
    pub confidence: f32,
}

/// ocr 输出。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrResult {
    /// 聊天对象名（慢环更新，快环沿用缓存）。
    pub chat_target: Option<String>,
    /// 输入框消息文本。
    pub draft_text: String,
    /// 全部文本块（含坐标、置信度）。
    pub blocks: Vec<TextBlock>,
    /// 聊天窗口最近对话摘要（仅慢环 OCR chat_window 区域时填充；快环沿用缓存）。
    /// 供语义判定提供上下文，消歧 L1 漏判 / L2 升级判定。
    pub chat_context: Option<String>,
}

impl OcrResult {
    /// 快环沿用慢环缓存的聊天对象名（§5.8）。
    pub fn with_cached_target(mut self, cached: Option<&str>) -> Self {
        if self.chat_target.is_none() {
            self.chat_target = cached.map(str::to_string);
        }
        self
    }

    /// 快环沿用慢环缓存的聊天上下文（§5.7-context）。
    pub fn with_cached_context(mut self, cached: Option<&str>) -> Self {
        if self.chat_context.is_none() {
            self.chat_context = cached.map(str::to_string);
        }
        self
    }
}
