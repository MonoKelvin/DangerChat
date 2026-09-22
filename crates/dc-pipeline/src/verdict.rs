//! 判定结果与判定槽位（设计文档 §4.1 `Verdict`、§2.2 `verdict_slot`）。
//!
//! 槽位语义：
//! - **无锁**（`arc-swap`）：钩子线程只做一次原子读，p99 < 1ms（NFR-02）。
//! - **时效**由 `stamp_ms` 判定，与 `Verdict.decided_at`（诊断用 `Instant`）解耦；
//!   §2.3 心跳的「零推理续期」只刷新 `stamp_ms`，不重建判定。
//! - **fail-open**：任何消费方读不到「新鲜且纪元匹配」的判定，一律放行（§2.1 原则 2）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;

/// 判定级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerdictLevel {
    Safe,
    Warn,
    Block,
}

impl VerdictLevel {
    /// 是否需要吞键 + 弹窗（§5.3：`Safe` 放行，`Warn`/`Block` 拦截）。
    pub fn is_dangerous(self) -> bool {
        matches!(self, VerdictLevel::Warn | VerdictLevel::Block)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VerdictLevel::Safe => "safe",
            VerdictLevel::Warn => "warn",
            VerdictLevel::Block => "block",
        }
    }
}

/// 危险分升 Block 所需的「阈值 + Δ」中的 Δ（§5.7）。
///
/// **0.25**（2026-09-18 由 0.15 上调）：在 1626 条标注语料（四段特征：草稿 ⊕ 对象 ⊕
/// 逐元素积 ⊕ 差）上实测，Δ=0.15 时 Block 线（formal 0.70 / casual 0.60）的误报率为
/// 2.4% / 6.9%；Δ=0.25 后 Block 线抬到 0.80 / 0.70，误报降至 **0.3% / 2.5%**，
/// 而 `Warn` 的覆盖范围（score ≥ 阈值）不变（召回 92% / 85%）。
/// 取向依据 §2.1 原则 2「宁漏勿阻」：硬拦截只留给高置信样本，其余一律 Warn 提示。
pub const BLOCK_MARGIN: f32 = 0.25;

/// 判定结果（sem 输出 / `verdict_slot` 内容）。
#[derive(Debug, Clone)]
pub struct Verdict {
    pub level: VerdictLevel,
    /// 0.0~1.0 危险概率。
    pub score: f32,
    /// 可读理由（FR-SEM-06）。
    pub reasons: Vec<String>,
    pub chat_target: Option<String>,
    /// 命中场景的展示名（正式/个人/自定义名；弹窗标签用，仅内存传递）。
    pub scene_name: Option<String>,
    /// 聊天窗口最近对话摘要（弹窗 UI 显示，仅内存传递）。
    pub chat_context: Option<String>,
    /// 草稿原文：仅内存传递供弹窗展示（FR-BRG-02）；日志只写指纹，永不落原文。
    pub draft_text: Option<String>,
    /// `draft_text` 的指纹（日志用）。
    pub draft_fingerprint: u64,
    /// 本判定对应的草稿纪元（§2.2 裁决依据）。
    pub draft_epoch: u64,
    pub decided_at: Instant,
}

impl Verdict {
    pub fn new(level: VerdictLevel, score: f32, reasons: Vec<String>) -> Self {
        Self {
            level,
            score,
            reasons,
            chat_target: None,
            scene_name: None,
            chat_context: None,
            draft_text: None,
            draft_fingerprint: 0,
            draft_epoch: 0,
            decided_at: Instant::now(),
        }
    }

    pub fn safe() -> Self {
        Self::new(VerdictLevel::Safe, 0.0, Vec::new())
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self::new(VerdictLevel::Block, 1.0, vec![reason.into()])
    }

    pub fn warn(reason: impl Into<String>) -> Self {
        Self::new(VerdictLevel::Warn, 0.5, vec![reason.into()])
    }

    /// L1 规则命中（§5.7）：命中即 Block，score 给名义值 1.0。
    pub fn from_rule(pattern: &str) -> Self {
        Self::new(
            VerdictLevel::Block,
            1.0,
            vec![format!("命中违禁词「{pattern}」")],
        )
    }

    /// L2 危险分（§5.7 阈值规则）：score 越过「阈值 + [`BLOCK_MARGIN`]」升 Block，
    /// 区间内 Warn。
    /// 恰好等于阈值即视为越线（≥，与文档「越过」的保守取向一致——正式场景宁严勿松）。
    /// 比较带 1e-6 容差：0.55+0.25 的 f32 是 0.80000001，不给容差会把整数分值随机降级。
    pub fn from_score(score: f32, threshold: f32) -> Self {
        const EPS: f32 = 1e-6;
        if score >= threshold + BLOCK_MARGIN - EPS {
            Self::new(VerdictLevel::Block, score, vec![])
        } else if score >= threshold - EPS {
            Self::new(VerdictLevel::Warn, score, vec![])
        } else {
            Self::new(VerdictLevel::Safe, score, vec![])
        }
    }

    pub fn with_epoch(mut self, epoch: u64) -> Self {
        self.draft_epoch = epoch;
        self
    }

    pub fn with_draft(mut self, text: impl Into<String>, fingerprint: u64) -> Self {
        self.draft_text = Some(text.into());
        self.draft_fingerprint = fingerprint;
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.chat_target = Some(target.into());
        self
    }

    /// 附带命中场景的展示名（弹窗标签用）。
    pub fn with_scene(mut self, scene: impl Into<String>) -> Self {
        self.scene_name = Some(scene.into());
        self
    }

    /// 附带上下文摘要（弹窗 UI 显示对话历史）。
    pub fn with_context(mut self, context: Option<String>) -> Self {
        self.chat_context = context;
        self
    }

    /// 判定本身是否在 TTL 内（基于自带的 `decided_at`；槽位另有 `stamp_ms` 用于续期）。
    pub fn fresh(&self, ttl: Duration) -> bool {
        self.decided_at.elapsed() < ttl
    }
}

/// 草稿文本指纹（xxhash 风格的 FNV-1a 64 位）。日志只写它，不写原文（隐私约定）。
pub fn draft_fingerprint(text: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x1000_0000_01b3;
    let mut hash = OFFSET;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// 无锁判定槽位（§2.2）。
#[derive(Debug, Default)]
pub struct VerdictSlot {
    inner: ArcSwapOption<Verdict>,
    /// 判定（或最近一次续期）的单调毫秒时间戳；0 = 空槽。
    stamp_ms: AtomicU64,
}

impl VerdictSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// 存入新判定。
    pub fn store(&self, verdict: Verdict, now_ms: u64) {
        self.inner.store(Some(Arc::new(verdict)));
        self.stamp_ms.store(now_ms.max(1), Ordering::SeqCst);
    }

    /// 零推理续期（§2.3 心跳：像素未变时只刷新时间戳）。返回是否续期成功（空槽返回 false）。
    pub fn refresh(&self, now_ms: u64) -> bool {
        if self.inner.load().is_some() {
            self.stamp_ms.store(now_ms.max(1), Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// 读取判定（不校验时效）。
    pub fn load(&self) -> Option<Arc<Verdict>> {
        self.inner.load_full()
    }

    /// 读取**新鲜**判定：无值或超出 TTL 时返回 `None`（调用方按 fail-open 处理）。
    pub fn load_fresh(&self, ttl: Duration, now_ms: u64) -> Option<Arc<Verdict>> {
        let verdict = self.inner.load_full()?;
        let stamp = self.stamp_ms.load(Ordering::SeqCst);
        if stamp == 0 || now_ms.saturating_sub(stamp) > ttl.as_millis() as u64 {
            return None;
        }
        Some(verdict)
    }

    /// 清空（任一 Stage 失败时按 fail-open 清槽，§5.8）。
    pub fn clear(&self) {
        self.inner.store(None);
        self.stamp_ms.store(0, Ordering::SeqCst);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.load().is_none()
    }

    /// 当前时间戳（测试与诊断用）。
    pub fn stamp_ms(&self) -> u64 {
        self.stamp_ms.load(Ordering::SeqCst)
    }
}
