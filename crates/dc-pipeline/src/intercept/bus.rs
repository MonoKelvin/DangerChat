//! 有界通道（§3.2）：钩子线程与 pipeline-worker / dc-alert 之间的唯一通信方式。
//!
//! **崩溃点全在这里的失败语义上**（§5.3 错误矩阵）：
//! - `trigger` 队列满 → 丢弃（下一个按键还会再触发），计数 + debug 日志；
//! - `alert` 队列满或通道关闭 → **改判放行（fail-open），绝不静默吞键**。
//!
//! 这也是为什么两个 bus 的 `try_send` 返回值必须被调用方检查——`#[must_use]` 强制这件事。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, TrySendError};

use super::keys::AlertAction;
use crate::verdict::Verdict;

/// 流水线触发类型（§5.8 `Trigger`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 输入变化触发的快环。
    Fast,
    /// 窗口/像素变化触发的慢环。
    Slow,
    /// 1.5s 心跳（仅 capture + pHash）。
    Heartbeat,
}

/// 触发总线（钩子线程 → pipeline-worker）。
#[derive(Debug)]
pub struct TriggerBus {
    tx: Sender<Trigger>,
    rx: Receiver<Trigger>,
    offered: AtomicU64,
    dropped: AtomicU64,
}

impl TriggerBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(capacity);
        Self {
            tx,
            rx,
            offered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// 提交一次触发。队列满即丢弃并返回 `false`（不阻塞钩子回调）。
    #[must_use]
    pub fn offer(&self, trigger: Trigger) -> bool {
        self.offered.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(trigger) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                // 合并语义：已有待处理触发，本次直接并进去
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// 内容键触发快环（§5.3）。
    #[must_use]
    pub fn offer_fast(&self) -> bool {
        self.offer(Trigger::Fast)
    }

    #[must_use]
    pub fn offer_slow(&self) -> bool {
        self.offer(Trigger::Slow)
    }

    pub fn try_recv(&self) -> Option<Trigger> {
        self.rx.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<Trigger> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn len(&self) -> usize {
        self.rx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    pub fn offered(&self) -> u64 {
        self.offered.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// 弹窗通道消息：后端推判定（弹窗），钩子代转弹窗动作（1/2/3/0）。
///
/// §5.3 伪码用的是同一个 `alert_bus`，这里忠实照做：单通道、双方向语义，
/// 由 dc-alert 侧按变体分派。
#[derive(Debug, Clone)]
pub enum AlertMessage {
    /// 拦截判定，请弹窗展示。
    Show(Arc<Verdict>),
    /// 弹窗存续期用户按下的动作键（钩子代转，因为弹窗不夺焦点）。
    Action(AlertAction),
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum AlertBusError {
    /// 队列满：dc-alert 来不及消费 → 调用方必须 fail-open。
    #[error("alert queue full")]
    Full,
    /// 通道关闭（前端退出）→ 调用方必须 fail-open。
    #[error("alert channel closed")]
    Disconnected,
}

/// 弹窗总线（钩子线程 ↔ dc-alert）。
#[derive(Debug)]
pub struct AlertBus {
    tx: Sender<AlertMessage>,
    rx: Receiver<AlertMessage>,
    shown: AtomicU64,
    actions: AtomicU64,
    failed: AtomicU64,
}

impl AlertBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(capacity);
        Self {
            tx,
            rx,
            shown: AtomicU64::new(0),
            actions: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }

    /// 请求弹窗。**失败即 fail-open**：调用方必须放行该次按键（§2.2）。
    pub fn request_show(&self, verdict: Arc<Verdict>) -> Result<(), AlertBusError> {
        match self.tx.try_send(AlertMessage::Show(verdict)) {
            Ok(()) => {
                self.shown.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                Err(AlertBusError::Full)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.failed.fetch_add(1, Ordering::Relaxed);
                Err(AlertBusError::Disconnected)
            }
        }
    }

    /// 代转弹窗动作。失败只丢一次快捷键（用户仍可点鼠标），不会造成吞键。
    pub fn send_action(&self, action: AlertAction) -> Result<(), AlertBusError> {
        match self.tx.try_send(AlertMessage::Action(action)) {
            Ok(()) => {
                self.actions.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(AlertBusError::Full),
            Err(TrySendError::Disconnected(_)) => Err(AlertBusError::Disconnected),
        }
    }

    pub fn try_recv(&self) -> Option<AlertMessage> {
        self.rx.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<AlertMessage> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn len(&self) -> usize {
        self.rx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    pub fn shown(&self) -> u64 {
        self.shown.load(Ordering::Relaxed)
    }

    pub fn actions(&self) -> u64 {
        self.actions.load(Ordering::Relaxed)
    }

    /// 入队失败次数（fail-open 次数，NFR-10 可观测）。
    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }
}
