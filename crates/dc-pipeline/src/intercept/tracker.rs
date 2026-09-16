//! 纪元跟踪（§2.2「草稿纪元 draft epoch」+ snooze）与前台跟踪（§5.3 去抖）。
//!
//! **本文件里的每个方法都必须能在钩子回调里 O(1) 调用**：全部是原子读或 CAS，
//! 没有任何锁、内存分配或系统调用。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// `draft_epoch` / `snooze_epoch` 两个原子量的唯一实现者（§2.2）。
#[derive(Debug)]
pub struct DraftTracker {
    epoch: AtomicU64,
    /// 「本次不再提示」静默纪元；仅等于当前纪元时静默。
    snooze_epoch: AtomicU64,
}

impl Default for DraftTracker {
    /// 初始：纪元 0、未 snooze（snooze_epoch 置 u64::MAX 表示「不等于任何当前纪元」）。
    fn default() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            snooze_epoch: AtomicU64::new(u64::MAX),
        }
    }
}

impl DraftTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// 内容修改键 / 输入法提交 → 纪元 +1。
    pub fn bump(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 「本次不再提示」：静默**当前**纪元；用户一改稿（纪元 +1）即自动恢复守护。
    pub fn snooze_current(&self) {
        self.snooze_epoch.store(self.epoch(), Ordering::SeqCst);
    }

    pub fn is_snoozed(&self) -> bool {
        self.snooze_epoch.load(Ordering::SeqCst) == self.epoch()
    }

    pub fn clear_snooze(&self) {
        self.snooze_epoch.store(u64::MAX, Ordering::SeqCst);
    }
}

/// 前台跟踪 + 去抖（§5.3「3s 去抖，防前台抖动」）。
///
/// 语义刻意做成**无定时器**：唤醒即时生效，挂起只在 `evaluate` 被调用且确认超过去抖窗口后才落地。
/// 因为 `evaluate` 会在每次按键与状态查询时被调用，去抖窗口内用户必然已经产生下一次交互，
/// 不需要额外的 timer 线程——这符合 ADR-09「后端核心不引入额外运行时」。
#[derive(Debug)]
pub struct ForegroundTracker {
    is_target: AtomicBool,
    /// 计划挂起的时刻；0 = 无计划。
    suspend_at_ms: AtomicU64,
    debounce_ms: u64,
}

impl ForegroundTracker {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            is_target: AtomicBool::new(false),
            suspend_at_ms: AtomicU64::new(0),
            debounce_ms,
        }
    }

    pub fn debounce_ms(&self) -> u64 {
        self.debounce_ms
    }

    /// 收到前台事件。目标回到前台 → 立即生效并取消挂起计划；目标离开 → 计划 `now + debounce`。
    pub fn note(&self, is_target: bool, now_ms: u64) {
        if is_target {
            self.is_target.store(true, Ordering::SeqCst);
            self.suspend_at_ms.store(0, Ordering::SeqCst);
        } else if self.is_target.load(Ordering::SeqCst) {
            self.suspend_at_ms
                .store(now_ms.saturating_add(self.debounce_ms), Ordering::SeqCst);
        }
    }

    /// 当前是否处于「目标前台」状态（内部顺带让到期的挂起计划落地）。
    pub fn evaluate(&self, now_ms: u64) -> bool {
        if !self.is_target.load(Ordering::SeqCst) {
            return false;
        }
        let at = self.suspend_at_ms.load(Ordering::SeqCst);
        if at != 0 && now_ms >= at {
            self.is_target.store(false, Ordering::SeqCst);
            self.suspend_at_ms.store(0, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// 立即挂起（忽略去抖）。用于暂停守护与测试。
    pub fn suspend_now(&self) {
        self.is_target.store(false, Ordering::SeqCst);
        self.suspend_at_ms.store(0, Ordering::SeqCst);
    }

    pub fn pending_suspend_at(&self) -> u64 {
        self.suspend_at_ms.load(Ordering::SeqCst)
    }
}
