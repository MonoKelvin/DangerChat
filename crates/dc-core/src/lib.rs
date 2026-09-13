//! # dc-core — 危信核心基座
//!
//! 本 crate 承载与业务无关的通用设施（设计文档 §5.1）：
//!
//! - [`config`]：配置中心，schema 注册 + 原子持久化 + 变更广播（FR-SYS-01）
//! - [`logging`]：日志中心与识别过程图片日志（FR-SYS-02/03/05）
//! - [`plugin`]：插件注册机制，模块统一生命周期契约（FR-SYS-04）
//! - [`model_store`]：ONNX 模型目录管理与导入校验
//!
//! 依赖方向（§3.3）：上层（dc-pipeline / dc-bridge）依赖本 crate；本 crate 不依赖任何上层，
//! 也不触碰 Win32（平台能力一律经 `dc-sys`）。

pub mod config;
pub mod logging;
pub mod model_store;
pub mod plugin;
pub mod time;

pub use config::{ConfigCenter, ConfigError, ConfigField, ConfigSnapshot, ConfigType, ConfigValue};
pub use logging::{
    ImageLogError, ImageLogSink, LogCenter, LogError, LogOptions, PruneReport, RunId,
};
pub use model_store::{ModelError, ModelInfo, ModelKind, ModelMeta, ModelStore};
pub use plugin::{
    MetricsRecorder, Module, ModuleContext, ModuleError, ModuleMetrics, PluginRegistry,
};
