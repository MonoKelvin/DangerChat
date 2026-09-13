//! 守护状态机（§2.4）：Active / Suspended / Paused / Cooldown。
//!
//! 刻意实现为**纯函数表**（不持任何状态），这样四态 × 全部事件的迁移矩阵可以穷举测试
//! （UT-INT-01），而 `Intercept` 只负责把事件喂进来、把结果存进 `AtomicU8`。
//!
//! ```text
//!                    ┌──────────── 前台失焦（去抖 3s 后）───────────┐
//!                    ▼                                            │
//!   [Suspended] ◀──── 前台失焦 ──── [Active] ──── AlertShown ──▶ [Cooldown]
//!        │                              │  ▲                        │
//!        │ Resume{false}                │  │ Resume{true}           │ AlertResolved / Timeout
//!        ▼                              ▼  │                        │
//!      [Paused] ◀────── Pause ────────────┴────────────────────────┤
//! ```
//!
//! 关键约定：
//! - `Suspended` 与 `Active` 是**前台派生态**；`Paused`（用户暂停）与 `Cooldown`（弹窗存续期）是
//!   **显式态**，前台事件不得改变它们（Paused 下切回微信不自动恢复守护，需用户 Resume）。
//! - Cooldown 期间再次 `AlertShown` 保持 Cooldown（§2.4「发送键一律吞但不重复弹窗」）。
//! - 挂起/暂停态收到 `AlertShown` 视为异常但不弹窗（保持原态）。

/// 守护状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardState {
    /// 目标前台：钩子裁决启用，流水线运行。
    Active,
    /// 目标非前台：钩子放行一切，模型卸载。
    Suspended,
    /// 用户暂停：等同未安装本软件。
    Paused,
    /// 弹窗存续期：数字键被截获代转，发送键吞掉但不重复弹窗。
    Cooldown,
}

impl GuardState {
    pub fn as_str(self) -> &'static str {
        match self {
            GuardState::Active => "active",
            GuardState::Suspended => "suspended",
            GuardState::Paused => "paused",
            GuardState::Cooldown => "cooldown",
        }
    }

    /// 钩子是否应该完全放行（不做任何裁决）。
    pub fn passes_everything(self) -> bool {
        matches!(self, GuardState::Suspended | GuardState::Paused)
    }

    /// `AtomicU8` 编码（也可用于状态持久化/上报）。
    pub fn to_u8(self) -> u8 {
        match self {
            GuardState::Active => 0,
            GuardState::Suspended => 1,
            GuardState::Paused => 2,
            GuardState::Cooldown => 3,
        }
    }

    /// `AtomicU8` 解码。
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => GuardState::Active,
            1 => GuardState::Suspended,
            2 => GuardState::Paused,
            _ => GuardState::Cooldown,
        }
    }
}

/// 驱动状态迁移的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardEvent {
    /// 目标程序回到前台（即时唤醒，不走去抖）。
    ForegroundActive,
    /// 目标程序失去前台（经去抖确认后才投递）。
    ForegroundInactive,
    /// 用户暂停守护（托盘菜单 / 快捷键，FR-SRC-07）。
    Pause,
    /// 用户恢复守护；是否回到 Active 取决于目标当时是否前台。
    Resume { target_active: bool },
    /// 拦截弹窗已展示（吞键成功后才投递）。
    AlertShown,
    /// 弹窗被处理（取消 / 返回编辑 / 仍然发送 / 本次不再提示）。
    AlertResolved,
    /// 弹窗倒计时到期（FR-BRG-04，默认不做任何动作）。
    AlertTimeout,
}

/// 迁移函数：`(当前态, 事件) → 新态`。
pub fn transition(from: GuardState, event: GuardEvent) -> GuardState {
    use GuardEvent as E;
    use GuardState as S;
    match (from, event) {
        // —— Active ——
        (S::Active, E::ForegroundInactive) => S::Suspended,
        (S::Active, E::ForegroundActive) => S::Active,
        (S::Active, E::Pause) => S::Paused,
        (S::Active, E::Resume { .. }) => S::Active,
        (S::Active, E::AlertShown) => S::Cooldown,
        (S::Active, E::AlertResolved | E::AlertTimeout) => S::Active,

        // —— Suspended ——
        (S::Suspended, E::ForegroundActive) => S::Active,
        (S::Suspended, E::ForegroundInactive) => S::Suspended,
        (S::Suspended, E::Pause) => S::Paused,
        (S::Suspended, E::Resume { .. }) => S::Suspended,
        // 挂起态不弹窗（Guard 只在 Active 裁决）
        (S::Suspended, E::AlertShown | E::AlertResolved | E::AlertTimeout) => S::Suspended,

        // —— Paused ——
        (
            S::Paused,
            E::Resume {
                target_active: true,
            },
        ) => S::Active,
        (
            S::Paused,
            E::Resume {
                target_active: false,
            },
        ) => S::Suspended,
        (S::Paused, E::Pause) => S::Paused,
        // 暂停期间前台怎么变都不影响恢复目标
        (S::Paused, E::ForegroundActive | E::ForegroundInactive) => S::Paused,
        (S::Paused, E::AlertShown | E::AlertResolved | E::AlertTimeout) => S::Paused,

        // —— Cooldown ——
        (S::Cooldown, E::AlertResolved | E::AlertTimeout) => S::Active,
        (S::Cooldown, E::AlertShown) => S::Cooldown,
        (S::Cooldown, E::ForegroundActive) => S::Cooldown,
        // 目标切走：弹窗失去意义，直接回挂起
        (S::Cooldown, E::ForegroundInactive) => S::Suspended,
        (S::Cooldown, E::Pause) => S::Paused,
        (S::Cooldown, E::Resume { .. }) => S::Cooldown,
    }
}
