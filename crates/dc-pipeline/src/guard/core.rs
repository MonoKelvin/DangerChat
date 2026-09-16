//! guard 编排纯核（§5.8 调度器）。
//!
//! 设计约束：
//! - **零线程**：本文件全部同步可测（UT-GRD-01~05 不 spawn worker）；
//! - **单飞**：运行中再来的触发被合并进 pending（Fast > Slow > Heartbeat），
//!   绝不排队——同一时刻最多一条流水线在跑（§2.3，DirectML 串行约束的根源）；
//! - **fail-open**：任一 Stage Recoverable → `verdict_slot.clear()`（下次按键必放行）；
//!   Fatal → 停用并告警（`fatal` 标志，后续触发直接跳过）。

use std::sync::Arc;

use dc_sys::{Hwnd, Rect};

use crate::clock::Clock;
use crate::contract::{
    LoopKind, OcrResult, PipelineContext, RegionLayout, Stage, StageError, WindowSnapshot,
};
use crate::intercept::Trigger;
use crate::verdict::{Verdict, VerdictSlot};

/// 布局缓存（§5.8 layout_cache）：窗口未变 + 年龄 < 30s 时快环复用。
pub struct LayoutCache {
    pub hwnd: Hwnd,
    pub rect: Rect,
    pub layout: RegionLayout,
    pub stored_at_ms: u64,
}

pub const LAYOUT_MAX_AGE_MS: u64 = 30_000;

impl LayoutCache {
    /// 复用校验三条件：hwnd 未变 && rect 未变 && 年龄 < 30s。
    pub fn reusable(&self, hwnd: Hwnd, rect: Rect, now_ms: u64) -> bool {
        self.hwnd == hwnd
            && self.rect == rect
            && now_ms.saturating_sub(self.stored_at_ms) < LAYOUT_MAX_AGE_MS
    }
}

/// 单飞合并状态：运行中到来的触发按优先级压在这里。
/// 优先序 Fast > Slow > Heartbeat（Fast 是用户正在打字，时效最敏感）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pending {
    pub fast: bool,
    pub slow: bool,
    pub heartbeat: bool,
}

impl Pending {
    pub fn offer(&mut self, t: Trigger) {
        match t {
            Trigger::Fast => self.fast = true,
            Trigger::Slow => self.slow = true,
            Trigger::Heartbeat => self.heartbeat = true,
        }
    }

    /// 取出最高优先级的待处理触发（取出即清该位）。
    pub fn take(&mut self) -> Option<Trigger> {
        if self.fast {
            self.fast = false;
            Some(Trigger::Fast)
        } else if self.slow {
            self.slow = false;
            Some(Trigger::Slow)
        } else if self.heartbeat {
            self.heartbeat = false;
            Some(Trigger::Heartbeat)
        } else {
            None
        }
    }
}

/// 心跳像素探测函数（注入 phash；测试可注入受控序列）。
pub type HeartbeatProbe = Arc<dyn Fn(&WindowSnapshot, &RegionLayout) -> u64 + Send + Sync>;

/// 编排器依赖集（泛型注入，测试用 MockStage）。
pub struct GuardCore<C, L, O, S>
where
    C: Stage<Input = crate::capture::CaptureRequest, Output = WindowSnapshot>,
    L: Stage<Input = WindowSnapshot, Output = RegionLayout>,
    O: Stage<Input = (WindowSnapshot, RegionLayout), Output = OcrResult>,
    S: Stage<Input = OcrResult, Output = Verdict>,
{
    pub capture: Arc<C>,
    pub layout: Arc<L>,
    pub ocr: Arc<O>,
    pub sem: Arc<S>,
    pub slot: Arc<VerdictSlot>,
    pub clock: Arc<dyn Clock>,
    /// 草稿纪元读取器（裁决依据；guard 每轮 store 时打上当前纪元）。
    pub draft_epoch: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// Fatal 后置位：后续触发全部跳过（fail-open 放行）。
    pub fatal: std::sync::atomic::AtomicBool,
    /// 慢环缓存的 chat_target（快环沿用，§5.8）。
    pub target_cache: std::sync::Mutex<Option<String>>,
    pub layout_cache: std::sync::Mutex<Option<LayoutCache>>,
    /// 一次 tick 内构造 PipelineContext 的工厂（run_id/日志/取消注入）。
    pub ctx_factory: Arc<dyn Fn(LoopKind) -> PipelineContext + Send + Sync>,
    /// 心跳 pHash（None = 心跳不做像素比较，直接升级 Slow——测试简化路径）。
    pub heartbeat_probe: Option<HeartbeatProbe>,
    pub last_phash: std::sync::Mutex<Option<u64>>,
}

/// 一次触发处理的结局（供测试断言与 worker 日志）。
#[derive(Debug, Clone, PartialEq)]
pub enum TickOutcome {
    /// 槽位已写入新判定。
    Stored,
    /// Recoverable → 槽位已清空（fail-open）。
    Cleared(String),
    /// Fatal → 已停用（本次起后续触发跳过）。
    Fatal(String),
    /// 触发被跳过（fatal 已置位 / 模型重载中 / 心跳像素未变仅续期）。
    Skipped(&'static str),
    /// 心跳像素未变 → 仅续期（零 Stage 推理调用）。
    Refreshed,
}

impl<C, L, O, S> GuardCore<C, L, O, S>
where
    C: Stage<Input = crate::capture::CaptureRequest, Output = WindowSnapshot>,
    L: Stage<Input = WindowSnapshot, Output = RegionLayout>,
    O: Stage<Input = (WindowSnapshot, RegionLayout), Output = OcrResult>,
    S: Stage<Input = OcrResult, Output = Verdict>,
{
    /// 处理一条触发（主入口；worker 循环与测试都调它）。
    pub fn run_trigger(
        &self,
        trigger: Trigger,
        request: crate::capture::CaptureRequest,
    ) -> TickOutcome {
        if self.fatal.load(std::sync::atomic::Ordering::Relaxed) {
            return TickOutcome::Skipped("fatal");
        }
        match trigger {
            Trigger::Fast => self.run_fast(request),
            Trigger::Slow => self.run_slow(request),
            Trigger::Heartbeat => self.run_heartbeat(request),
        }
    }

    fn run_fast(&self, request: crate::capture::CaptureRequest) -> TickOutcome {
        // 布局缓存可用性（§5.8）：不可用 → 升级 Slow
        let cache_ok = {
            let guard = self.layout_cache.lock().ok();
            match guard {
                Some(cache) if cache.is_some() => {
                    // hwnd/rect 由调用方从 request 携带；快环请求即当前窗口
                    let c = cache.as_ref().unwrap();
                    c.reusable(request.hwnd, c.rect, self.clock.now_ms())
                }
                _ => false,
            }
        };
        if !cache_ok {
            return self.run_slow(request);
        }

        let ctx = (self.ctx_factory)(LoopKind::Fast);
        let snap = match self.capture.process(request, &ctx) {
            Ok(s) => s,
            Err(e) => return self.stage_failed(e),
        };
        let layout = {
            let guard = self.layout_cache.lock().ok();
            guard
                .and_then(|c| c.as_ref().map(|c| c.layout.clone()))
                .unwrap_or_else(|| RegionLayout {
                    regions: Vec::new(),
                    layout_epoch: 0,
                    inferred_at: std::time::Instant::now(),
                })
        };
        let ocr = match self.ocr.process((snap, layout.input_only()), &ctx) {
            Ok(o) => o,
            Err(e) => return self.stage_failed(e),
        };
        let cached_target = self.target_cache.lock().ok().and_then(|t| t.clone());
        let ocr = ocr.with_cached_target(cached_target.as_deref());
        let verdict = match self.sem.process(ocr, &ctx) {
            Ok(v) => v,
            Err(e) => return self.stage_failed(e),
        };
        self.store(verdict);
        TickOutcome::Stored
    }

    fn run_slow(&self, request: crate::capture::CaptureRequest) -> TickOutcome {
        let ctx = (self.ctx_factory)(LoopKind::Slow);
        let snap = match self.capture.process(request, &ctx) {
            Ok(s) => s,
            Err(e) => return self.stage_failed(e),
        };
        let layout = match self.layout.process(snap.clone(), &ctx) {
            Ok(l) => l,
            Err(e) => return self.stage_failed(e),
        };
        let window_rect = snap.window_rect;
        let ocr = match self.ocr.process((snap, layout.clone()), &ctx) {
            Ok(o) => o,
            Err(e) => return self.stage_failed(e),
        };
        if let Ok(mut t) = self.target_cache.lock() {
            *t = ocr.chat_target.clone();
        }
        if let Ok(mut c) = self.layout_cache.lock() {
            *c = Some(LayoutCache {
                hwnd: request.hwnd,
                rect: window_rect,
                layout: layout.clone(),
                stored_at_ms: self.clock.now_ms(),
            });
        }
        let verdict = match self.sem.process(ocr, &ctx) {
            Ok(v) => v,
            Err(e) => return self.stage_failed(e),
        };
        self.store(verdict);
        TickOutcome::Stored
    }

    fn run_heartbeat(&self, request: crate::capture::CaptureRequest) -> TickOutcome {
        // 1.5s 心跳：capture + chat_target ROI pHash（§2.3）。
        // 无探测函数（测试简化）或无布局 → 直接触发 Slow（拿不到 ROI）。
        let Some(probe) = &self.heartbeat_probe else {
            return self.run_slow(request);
        };
        let ctx = (self.ctx_factory)(LoopKind::Heartbeat);
        let snap = match self.capture.process(request, &ctx) {
            Ok(s) => s,
            Err(e) => return self.stage_failed(e),
        };
        let layout = {
            let guard = self.layout_cache.lock().ok();
            guard.and_then(|c| c.as_ref().map(|c| c.layout.clone()))
        };
        let Some(layout) = layout else {
            return self.run_slow(request);
        };
        let hash = probe(&snap, &layout);
        let unchanged = match self.last_phash.lock() {
            Ok(mut last) => {
                let same = *last == Some(hash);
                *last = Some(hash);
                same
            }
            Err(_) => false,
        };
        if unchanged {
            // 零推理续期：decided_at 仍是旧值，stamp 刷新使 TTL 不超时（§2.3）
            let _ = self.slot.refresh(self.clock.now_ms());
            TickOutcome::Refreshed
        } else {
            self.run_slow_from_snapshot(snap)
        }
    }

    fn run_slow_from_snapshot(&self, snap: WindowSnapshot) -> TickOutcome {
        let ctx = (self.ctx_factory)(LoopKind::Slow);
        let layout = match self.layout.process(snap.clone(), &ctx) {
            Ok(l) => l,
            Err(e) => return self.stage_failed(e),
        };
        let window_rect = snap.window_rect;
        let ocr = match self.ocr.process((snap, layout.clone()), &ctx) {
            Ok(o) => o,
            Err(e) => return self.stage_failed(e),
        };
        if let Ok(mut t) = self.target_cache.lock() {
            *t = ocr.chat_target.clone();
        }
        if let Ok(mut c) = self.layout_cache.lock() {
            *c = Some(LayoutCache {
                hwnd: Hwnd(0),
                rect: window_rect,
                layout: layout.clone(),
                stored_at_ms: self.clock.now_ms(),
            });
        }
        let verdict = match self.sem.process(ocr, &ctx) {
            Ok(v) => v,
            Err(e) => return self.stage_failed(e),
        };
        self.store(verdict);
        TickOutcome::Stored
    }

    fn stage_failed(&self, e: StageError) -> TickOutcome {
        match e {
            StageError::Recoverable(msg) => {
                // fail-open：槽位清空 → 下次按键 load_fresh  miss → 放行
                self.slot.clear();
                TickOutcome::Cleared(msg)
            }
            StageError::Fatal(msg) => {
                self.fatal.store(true, std::sync::atomic::Ordering::Relaxed);
                self.slot.clear();
                TickOutcome::Fatal(msg)
            }
            StageError::Cancelled => TickOutcome::Skipped("cancelled"),
        }
    }

    fn store(&self, verdict: Verdict) {
        let epoch = (self.draft_epoch)();
        tracing::info!(
            level = verdict.level.as_str(),
            score = verdict.score,
            draft = verdict.draft_text.as_deref().unwrap_or(""),
            "流水线判定已入槽"
        );
        self.slot
            .store(verdict.with_epoch(epoch), self.clock.now_ms());
    }
}
