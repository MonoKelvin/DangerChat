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

use crate::contract::{LoopKind, Module, PipelineContext, RegionLayout, WindowSnapshot};
use crate::guard::core::HeartbeatProbe;
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
        // 目标是否前台（挂起态卸载判据，§2.4）。由调用方从 Arc<Intercept> 构造：
        // { let i = intercept.clone(); move || i.target_active() }
        target_active: Arc<dyn Fn() -> bool + Send + Sync>,
        // 判定入槽回调（fail-closed 补判闭环，§2.2 修订）。由��调用方从 Arc<Intercept> 构造：
        // { let i = intercept.clone(); move |v| i.on_analysis_ready(v) }
        on_verdict_stored: Arc<dyn Fn(&crate::verdict::Verdict) + Send + Sync>,
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

        let clock = intercept.clock_arc(); // 复用 Intercept 的时钟：保证 TTL stamp 与检查使用同一基线
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

        // 心跳 pHash 探针（§2.3）：裁 chat_target ROI（拿不到则整窗）算感知哈希。
        // **必须接线**——缺它心跳会退化成每 1.5s 一次全链路慢环，占满 worker，
        // 快环判定被饿死，导致回车迟钝 + 同消息时而漏弹。
        let heartbeat_probe: HeartbeatProbe =
            Arc::new(|snap: &WindowSnapshot, layout: &RegionLayout| -> u64 {
                let (iw, ih) = (snap.image.width(), snap.image.height());
                // ROI：优先聊天对象区；缺失时退整窗（心跳只需回答「聊天对象是否变了」）
                let (x, y, w, h) = match layout.rect_of(crate::contract::TAG_CHAT_TARGET) {
                    Some(r) => {
                        let x = r.x.max(0) as u32;
                        let y = r.y.max(0) as u32;
                        (
                            x,
                            y,
                            r.w.min(iw.saturating_sub(x)),
                            r.h.min(ih.saturating_sub(y)),
                        )
                    }
                    None => (0, 0, iw, ih),
                };
                if w == 0 || h == 0 {
                    return phash::phash(&snap.image);
                }
                let roi = image::imageops::crop_imm(&snap.image, x, y, w, h).to_image();
                phash::phash(&roi)
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
            context_cache: std::sync::Mutex::new(None),
            layout_cache: std::sync::Mutex::new(None),
            ctx_factory,
            heartbeat_probe: Some(heartbeat_probe),
            last_phash: std::sync::Mutex::new(None),
            on_verdict_stored,
        });

        let worker_cancel = crate::contract::CancellationToken::new();
        let worker = spawn_worker(
            Arc::clone(&core),
            intercept.triggers_arc(),
            worker_cancel.clone(),
            HEARTBEAT,
            target_hwnd,
            target_active,
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
    target_active: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<std::thread::JoinHandle<()>, String> {
    std::thread::Builder::new()
        .name("pipeline-worker".into())
        .spawn(move || {
            let mut pending = Pending::default();
            let mut next_heartbeat = std::time::Instant::now() + heartbeat;
            // 惰性加载（§2.4）：sem 模型 init 时**未加载**（省常驻内存），故初始视为「未激活」，
            // 首次进入前台由下面的巡检/触发路径按需装入。仅在前台态跳变时装/卸，避免每轮 IO。
            let mut was_active = false;
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
                    // 前台态巡检（每个触发都查一次，O(1) 原子读）：离开前台 → 卸载释放内存；
                    // 在前台 → 确保装入（~600ms，装入完成前该轮 fail-open 放行）。
                    // 放在触发处理**之前**，保证用户一打字（Fast 触发）即触发按需装入，
                    // 不必等下一次 1.5s 心跳。
                    //
                    // 为何前台时**每轮**调 ensure_loaded 而非仅在跳变时调：ensure_loaded 幂等，
                    // 已加载则只取 read 锁判 is_some 即返回（无 IO）；未加载（含**上一轮装入失败**）
                    // 才真正装入。旧实现用 was_active 门控 ensure，一旦某次装入失败便把 was_active
                    // 置真，此后前台态不再跳变 → 永不重试，模型永久停在「已卸载」——用户实测
                    // 「切走目标再切回后语义失效、提示模型未加载」即此故障（一次性失败无从恢复）。
                    let active = target_active();
                    if active {
                        core.sem.ensure_loaded();
                    } else if was_active {
                        core.sem.unload_model();
                    }
                    was_active = active;
                    if t == Trigger::Heartbeat {
                        next_heartbeat = std::time::Instant::now() + heartbeat;
                    }
                    let request = crate::capture::CaptureRequest { hwnd: target_hwnd, roi: None };
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
