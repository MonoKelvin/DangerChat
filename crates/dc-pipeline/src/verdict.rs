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

/// 判定结果（sem 输出 / `verdict_slot` 内容）。
#[derive(Debug, Clone)]
pub struct Verdict {
    pub level: VerdictLevel,
    /// 0.0~1.0 危险概率。
    pub score: f32,
    /// 可读理由（FR-SEM-06）。
    pub reasons: Vec<String>,
    pub chat_target: Option<String>,
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
