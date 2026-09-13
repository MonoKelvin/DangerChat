//! 插件注册机制与模块统一生命周期契约（FR-SYS-04，设计文档 §4）。
//!
//! `Module` 是所有功能模块的共同基座（id / 配置 schema / init / shutdown / metrics）。
//! 流水线阶段 trait `Stage: Module` 定义在 `dc-pipeline::contract`——它需要引用
//! `PipelineContext`、`StageError` 这些流水线专属类型，放在流水线 crate 里更自然；
//! 而注册表本身属于核心设施（§5.1 `PluginRegistry` 位于 dc-core），故 `Module` 契约随之落在本 crate。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{ConfigCenter, ConfigError, ConfigField, ConfigSnapshot};
use crate::model_store::ModelStore;

/// 模块初始化上下文：模块所需的一切外部依赖都从这里注入，模块不读全局状态（§4 约定）。
pub struct ModuleContext {
    pub config: Arc<ConfigSnapshot>,
    pub models: Arc<ModelStore>,
    /// 日志目录（模块自有的中间产物写在这里的子目录下）。
    pub log_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("init failed: {0}")]
    Init(String),
    #[error("shutdown failed: {0}")]
    Shutdown(String),
    /// 致命错误：模块停用 + 前端告警（与 `StageError::Fatal` 同构）
    #[error("fatal: {0}")]
    Fatal(String),
}

/// 运行期指标快照（§4 `ModuleMetrics`）。
///
/// 耗时以**纳秒**累计：按键路径的单次回调是亚微秒级（实测 ~200ns），
/// 按微秒累计会被整数截断成 0，指标就失去意义。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleMetrics {
    pub invocations: u64,
    pub failures: u64,
    pub total_duration_ns: u64,
    pub max_duration_ns: u64,
}

impl ModuleMetrics {
    pub fn avg_duration_ns(&self) -> u64 {
        self.total_duration_ns
            .checked_div(self.invocations)
            .unwrap_or(0)
    }

    pub fn avg_duration_us(&self) -> u64 {
        self.avg_duration_ns() / 1_000
    }
}

/// 线程安全指标累加器：模块持有一份，`metrics(&self)` 时直接出快照，无需额外锁。
#[derive(Debug, Default)]
pub struct MetricsRecorder {
    invocations: AtomicU64,
    failures: AtomicU64,
    total_ns: AtomicU64,
    max_ns: AtomicU64,
}

impl MetricsRecorder {
    pub fn record(&self, duration: Duration) {
        let ns = duration.as_nanos().min(u64::MAX as u128) as u64;
        self.invocations.fetch_add(1, Ordering::Relaxed);
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ModuleMetrics {
        ModuleMetrics {
            invocations: self.invocations.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            total_duration_ns: self.total_ns.load(Ordering::Relaxed),
            max_duration_ns: self.max_ns.load(Ordering::Relaxed),
        }
    }
}

/// 功能模块契约。实现者必须 `Send + Sync`（跨线程共享，§3.2）。
pub trait Module: Send + Sync {
    fn id(&self) -> &'static str;

    /// 向配置中心声明本模块的配置项（键、类型、默认值、说明）。
    fn config_schema(&self) -> Vec<ConfigField>;

    fn init(&mut self, ctx: &ModuleContext) -> Result<(), ModuleError>;

    fn shutdown(&mut self) -> Result<(), ModuleError>;

    fn metrics(&self) -> ModuleMetrics {
        ModuleMetrics::default()
    }
}

/// 模块实例注册表，由 guard 编排器在启动时装配（§5.1）。
#[derive(Default)]
pub struct PluginRegistry {
    modules: Vec<Box<dyn Module>>,
    inited: Vec<&'static str>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册模块。id 重复 → panic（装配期开发错误，§5.1 同一处置）。
    pub fn register(&mut self, module: Box<dyn Module>) {
        let id = module.id();
        assert!(
            !self.modules.iter().any(|m| m.id() == id),
            "模块 id `{id}` 重复注册"
        );
        self.modules.push(module);
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.modules.iter().map(|m| m.id()).collect()
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn get_mut<'a>(&'a mut self, id: &str) -> Option<&'a mut (dyn Module + 'a)> {
        let idx = self.modules.iter().position(|m| m.id() == id)?;
        Some(self.modules[idx].as_mut())
    }

    /// 把全部模块的配置 schema 注册进配置中心（顺序无关，键冲突由 ConfigCenter 处置）。
    pub fn register_configs(&self, center: &ConfigCenter) -> Result<(), ConfigError> {
        for module in &self.modules {
            center.register_module(module.id(), module.config_schema())?;
        }
        Ok(())
    }

    /// 按注册顺序初始化。单个模块失败不影响其余模块（失败列表返回给调用方做前端告警）。
    pub fn init_all(&mut self, ctx: &ModuleContext) -> Vec<(&'static str, ModuleError)> {
        let mut failures = Vec::new();
        for module in &mut self.modules {
            let id = module.id();
            match module.init(ctx) {
                Ok(()) => self.inited.push(id),
                Err(err) => {
                    tracing::error!(module = id, error = %err, "模块初始化失败，跳过");
                    failures.push((id, err));
                }
            }
        }
        failures
    }

    /// 逆序关闭（后初始化者先关闭，依赖关系自然倒置）。
    pub fn shutdown_all(&mut self) -> Vec<(&'static str, ModuleError)> {
        let mut failures = Vec::new();
        for module in self.modules.iter_mut().rev() {
            let id = module.id();
            if !self.inited.contains(&id) {
                continue;
            }
            if let Err(err) = module.shutdown() {
                tracing::error!(module = id, error = %err, "模块关闭失败");
                failures.push((id, err));
            }
        }
        self.inited.clear();
        failures
    }

    pub fn metrics(&self) -> Vec<(&'static str, ModuleMetrics)> {
        self.modules.iter().map(|m| (m.id(), m.metrics())).collect()
    }
}
