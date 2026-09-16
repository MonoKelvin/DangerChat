//! 流水线编排器（设计文档 §5.8）：worker 线程 + 触发合并 + 心跳。
//!
//! 线程模型（ADR-09）：`Guard::spawn` 起一条 pipeline-worker（std::thread），
//! 在 TriggerBus 与心跳定时之间 select；单线程顺序执行天然满足 DirectML
//! 串行约束。模型装卸载生命周期（Suspended 卸载/Active 重载）挂 M6 壳装配
//! （届时才有暂停/恢复的调用方），本层提供 `reload()` 语义。

pub mod core;
pub mod phash;

use std::sync::Arc;
use std::time::Duration;

use dc_core::ConfigField;
use dc_sys::{Hwnd, SysApi};

use crate::clock::MonoClock;
use crate::contract::{LoopKind, Module, PipelineContext};
use crate::intercept::{Intercept, Trigger};
use crate::layout::LayoutStage;
use crate::ocr::OcrStage;
use crate::sem::SemStage;

pub use core::{GuardCore, LayoutCache, Pending, TickOutcome};
pub use phash::{hamming, phash, unchanged};

/// 心跳周期（§2.3：1.5s 固定，仅 Active 态）。
pub const HEARTBEAT: Duration = Duration::from_millis(1500);

/// 生产装配：四真 Stage + intercept 接线 + worker。
pub struct Guard {
    worker: Option<std::thread::JoinHandle<()>>,
    cancel: crate::contract::CancellationToken,
    core: Arc<GuardCore<crate::capture::CaptureStage, LayoutStage, OcrStage, SemStage>>,
    stages: StagesHandle,
    target_hwnd: Hwnd,
}

#[allow(dead_code)] // capture/sem 为生命周期持有（guard 存活即 Stage 存活）
struct StagesHandle {
    /// 持有全部 Stage 引用（含只读使用的 capture/sem）——Guard 存活即 Stage 存活。
    capture: Arc<crate::capture::CaptureStage>,
    layout: Arc<LayoutStage>,
    ocr: Arc<OcrStage>,
    sem: Arc<SemStage>,
    mctx_factory: Arc<dyn Fn() -> dc_core::ModuleContext + Send + Sync>,
}

impl Guard {
    /// 装配 + 起 worker。推理 Stage init 失败**不拒绝装配**（fail-open 语义：
    /// 模块停用，触发跳过，宁漏勿阻）；装配期只拒绝线程创建失败。
    pub fn spawn(
        intercept: &Intercept,
        sys: Arc<dyn SysApi>,
        mctx_factory: impl Fn() -> dc_core::ModuleContext + Send + Sync + 'static,
        config: Arc<dc_core::ConfigSnapshot>,
        target_hwnd: Hwnd,
    ) -> Result<Self, String> {
        let capture = Arc::new(crate::capture::CaptureStage::new(sys));
        let mctx = mctx_factory();
        let _mctx_log_root = mctx.log_dir.clone();
        let image_store_ref = mctx.image_store.clone();

        let (layout, ocr, sem) = {
            let mut layout = LayoutStage::new();
            if let Err(e) = layout.init(&mctx) {
                tracing::warn!(error = %e, "layout init 失败（fail-open）");
            }
            let mut ocr = OcrStage::new();
            if let Err(e) = ocr.init(&mctx) {
                tracing::warn!(error = %e, "ocr init 失败（fail-open）");
            }
            let mut sem = SemStage::empty();
            if let Err(e) = sem.init(&mctx) {
                tracing::warn!(error = %e, "sem init 失败（fail-open）");
            }
            (Arc::new(layout), Arc::new(ocr), Arc::new(sem))
        };

        let clock: Arc<dyn crate::clock::Clock> = Arc::new(MonoClock::new());
        let draft_epoch = {
            let tracker = intercept.tracker_arc();
            Arc::new(move || tracker.epoch()) as Arc<dyn Fn() -> u64 + Send + Sync>
        };

        let run_seed = std::sync::atomic::AtomicU64::new(0);
        let cfg = Arc::clone(&config);
        let ctx_factory = Arc::new(move |kind: LoopKind| {
            let n = run_seed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let run_id = dc_core::RunId::from_raw(format!("run-{n:08}"));
            let image_sink = match &image_store_ref {
                Some(store) => dc_core::ImageLogSink::from_store(Arc::clone(store)),
                None => dc_core::ImageLogSink::noop(),
            };
            PipelineContext::new(run_id, kind, image_sink, Arc::clone(&cfg))
        });

        let core = Arc::new(GuardCore {
            capture: Arc::clone(&capture),
            layout: Arc::clone(&layout),
            ocr: Arc::clone(&ocr),
            sem: Arc::clone(&sem),
            slot: intercept.slot_arc(),
            clock,
            draft_epoch,
            fatal: std::sync::atomic::AtomicBool::new(false),
            target_cache: std::sync::Mutex::new(None),
            layout_cache: std::sync::Mutex::new(None),
            ctx_factory,
            heartbeat_probe: None,
            last_phash: std::sync::Mutex::new(None),
        });

        let worker_cancel = crate::contract::CancellationToken::new();
        let worker = spawn_worker(
            Arc::clone(&core),
            intercept.triggers_arc(),
            worker_cancel.clone(),
            HEARTBEAT,
            target_hwnd,
        )?;

        Ok(Self {
            worker: Some(worker),
            cancel: worker_cancel,
            core,
            stages: StagesHandle {
                capture,
                layout,
                ocr,
                sem,
                mctx_factory: Arc::new(mctx_factory),
            },
            target_hwnd,
        })
    }

    pub fn core(
        &self,
    ) -> &Arc<GuardCore<crate::capture::CaptureStage, LayoutStage, OcrStage, SemStage>> {
        &self.core
    }

    /// 装配时的目标窗口（泵定位弹窗用）。
    pub fn target_hwnd(&self) -> Hwnd {
        self.target_hwnd
    }

    /// 模型重载（§5.8 生命周期：唤醒/导入新模型后调用；重载完成前触发走 fail-open）。
    ///
    /// 注意：worker 持 Stage 的共享引用，重载需 `Arc::get_mut`——调用方必须
    /// 独占（`&mut self` 或先 `stop()`）；M6 壳装配时按此约定接线。
    pub fn reload_models(&mut self) {
        let mctx = (self.stages.mctx_factory)();
        if let Some(layout) = Arc::get_mut(&mut self.stages.layout) {
            if let Err(e) = layout.init(&mctx) {
                tracing::warn!(error = %e, "layout 重载失败");
            }
        }
        if let Some(ocr) = Arc::get_mut(&mut self.stages.ocr) {
            if let Err(e) = ocr.init(&mctx) {
                tracing::warn!(error = %e, "ocr 重载失败");
            }
        }
        // sem 不热重载（规则/头热更新走 M6 set_config → 快照重建；模型本体常驻）
    }

    /// 规则热重载：保存 rules/contacts/scenes 后前端调用此方法让 sem 重新加载。
    pub fn reload_rules(&self, rules_path: &str, contacts_path: &str, scenes_path: &str) {
        self.stages
            .sem
            .reload_rules_and_contacts(rules_path, contacts_path, scenes_path);
    }

    pub fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_worker(
    core: Arc<GuardCore<crate::capture::CaptureStage, LayoutStage, OcrStage, SemStage>>,
    bus: Arc<crate::intercept::TriggerBus>,
    cancel: crate::contract::CancellationToken,
    heartbeat: Duration,
    target_hwnd: Hwnd,
) -> Result<std::thread::JoinHandle<()>, String> {
    std::thread::Builder::new()
        .name("pipeline-worker".into())
        .spawn(move || {
            let mut pending = Pending::default();
            let mut next_heartbeat = std::time::Instant::now() + heartbeat;
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                let timeout = next_heartbeat.saturating_duration_since(std::time::Instant::now());
                match bus.recv_timeout(timeout) {
                    Some(t) => pending.offer(t),
                    None => pending.offer(Trigger::Heartbeat),
                }
                while let Some(t) = bus.try_recv() {
                    pending.offer(t);
                }
                while let Some(t) = pending.take() {
                    if cancel.is_cancelled() {
                        return;
                    }
                    if t == Trigger::Heartbeat {
                        next_heartbeat = std::time::Instant::now() + heartbeat;
                    }
                    let request = crate::capture::CaptureRequest { hwnd: target_hwnd };
                    let outcome = core.run_trigger(t, request);
                    if let TickOutcome::Fatal(msg) = &outcome {
                        tracing::error!(error = %msg, stage = "guard", "Fatal → 停用（后续触发跳过）");
                    }
                }
            }
        })
        .map_err(|e| format!("pipeline-worker 启动失败：{e}"))
}

/// guard 的编排配置（`pipeline.*` 前缀；intercept 已占用 `guard.*`）。
///
/// 无配置项：心跳周期为编译期常量 `HEARTBEAT`（§2.3 固定 1.5s，
/// 曾有 `pipeline.heartbeat_ms` 配置但从未被读取，已删）。
pub fn config_schema() -> Vec<ConfigField> {
    Vec::new()
}
