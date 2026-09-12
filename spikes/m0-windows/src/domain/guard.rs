//! 发送事务状态机与一次性许可。
//!
//! 纯逻辑：不依赖 Windows、不分配堆内存、不加锁，可在任意平台穷举单测。
//! 该结构由钩子线程独占访问（`LowLevelKeyboardProc` 始终在安装钩子的线程上下文执行），
//! 因此无需互斥量即可保证快路径 O(1)。
//!
//! 对应文档：`docs/02_危信v1.0_技术架构与接口设计.md` §8-§9，
//! `docs/01_危信v1.0_产品需求与验收规格.md` FR-02/FR-03/FR-08。

use super::keys::{Modifiers, PhysicalKey, Shortcut};

/// 许可最长存活时间：10 秒。
pub const PERMIT_MAX_LIFETIME_NANOS: u64 = 10_000_000_000;
/// 消费许可时要求的快照新鲜度上限：300 毫秒。
pub const SNAPSHOT_FRESHNESS_NANOS: u64 = 300_000_000;

/// 同时可跟踪的已抑制物理按键数量。Enter 变体最多 2 个，留冗余但不分配。
const SUPPRESSED_SLOTS: usize = 4;

/// 钩子回调对单次按键的裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 透传给系统与目标程序。
    Pass,
    /// 阻止本次按键（回调返回非零）。
    Suppress,
    /// 消费一次性许可并透传该物理按键。
    ConsumePermit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Down { repeat: bool },
    Up,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key: PhysicalKey,
    pub action: KeyAction,
    /// 由其他进程合成的事件。危信自身从不注入。
    pub injected: bool,
}

/// 目标窗口实例身份。HWND 会被复用，必须与 PID 和进程启动时间一起比较。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TargetInstance {
    pub root_hwnd: u64,
    pub pid: u32,
    pub process_start_time: u64,
}

/// 事务与许可绑定的上下文指纹。均不含聊天文本，只是不可逆摘要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextFingerprint {
    pub context_generation: u64,
    pub scene_fingerprint: u64,
    pub draft_fingerprint: u64,
}

/// 后台复核发布的新鲜快照。许可消费时必须同时满足代次一致与新鲜度。
#[derive(Debug, Clone, Copy)]
pub struct VerifiedSnapshot {
    pub fingerprint: ContextFingerprint,
    pub captured_at: u64,
}

impl VerifiedSnapshot {
    pub fn is_fresh_at(&self, now: u64) -> bool {
        now.saturating_sub(self.captured_at) <= SNAPSHOT_FRESHNESS_NANOS
    }
}

/// 一次性许可。绑定窗口实例、上下文指纹、快捷键与策略版本，只能被消费一次。
#[derive(Debug, Clone, Copy)]
pub struct Permit {
    pub permit_id: u64,
    pub target: TargetInstance,
    pub fingerprint: ContextFingerprint,
    pub shortcut: Shortcut,
    pub policy_version: u32,
    pub issued_at: u64,
    pub reason: PermitReason,
}

/// 许可来源。三类必须分别记录，不能混为“安全”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermitReason {
    /// R0：内容与场景匹配且证据充分。
    LowRisk,
    /// 用户在风险窗口选择“仍要发送”后经复核签发。
    RiskOverride,
    /// 检测无法完成时用户明确选择“本次跳过检测”。
    UnavailableSkip,
}

impl Permit {
    pub fn expires_at(&self) -> u64 {
        self.issued_at.saturating_add(PERMIT_MAX_LIFETIME_NANOS)
    }
}

/// 发送事务。全局同时只允许一个。
#[derive(Debug, Clone, Copy)]
pub struct SendTransaction {
    pub transaction_id: u64,
    pub target: TargetInstance,
    pub fingerprint: ContextFingerprint,
    pub user_activity_generation: u64,
    pub shortcut: Shortcut,
    pub created_at: u64,
}

/// 事务状态。与 `docs/02` §8 的状态图一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Disarmed,
    ArmedIdle,
    Held,
    Capturing,
    Analyzing,
    Revalidating,
    AwaitingReview,
    RevalidatingOverride,
    PermitReady,
    Unavailable,
}

impl State {
    /// 是否处于“已持有用户按键”的非空闲状态。
    ///
    /// 这些状态下任何新一轮发送键都必须被抑制，且不得创建第二个事务。
    pub fn holds_transaction(self) -> bool {
        !matches!(self, State::Disarmed | State::ArmedIdle)
    }
}

/// 事务终止原因，用于诊断计数（不含任何内容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    UserCancelled,
    ContextChanged,
    TargetLost,
    Expired,
    Shutdown,
}

/// 钩子线程发给协调线程的请求（经有界队列传递，回调内不阻塞）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardRequest {
    /// 已抑制一次发送键，请开始捕获与分析。
    StartTransaction { transaction_id: u64 },
    /// 许可已被物理按键消费。
    PermitConsumed { permit_id: u64 },
    /// 许可存在但快照过期/代次不符，已抑制本次按键，请重新复核。
    RecheckRequired { permit_id: u64 },
    /// 用户在分析期间继续编辑，旧结果失效。
    UserActivity { generation: u64 },
}

/// 已抑制 keydown 的物理按键表，用于配对抑制其 repeat 与 keyup。
#[derive(Debug, Default)]
struct SuppressedKeys {
    slots: [Option<PhysicalKey>; SUPPRESSED_SLOTS],
}

impl SuppressedKeys {
    fn insert(&mut self, key: PhysicalKey) {
        if self.contains(key) {
            return;
        }
        for slot in self.slots.iter_mut() {
            if slot.is_none() {
                *slot = Some(key);
                return;
            }
        }
        // 槽位耗尽：不覆盖已有条目。该情形下守卫必然处于非空闲状态，
        // 发送键仍会被状态判定抑制，不会出现孤立 keyup。
    }

    fn contains(&self, key: PhysicalKey) -> bool {
        self.slots.contains(&Some(key))
    }

    fn remove(&mut self, key: PhysicalKey) -> bool {
        for slot in self.slots.iter_mut() {
            if *slot == Some(key) {
                *slot = None;
                return true;
            }
        }
        false
    }

    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

/// 键盘守卫核心。由钩子线程独占。
pub struct GuardCore {
    state: State,
    armed_target: Option<TargetInstance>,
    shortcut: Shortcut,
    policy_version: u32,

    /// 目标是否为前台且系统会话可用。
    target_foreground: bool,
    /// 输入法是否处于组合/候选状态。为 true 时一律透传。
    ime_composing: bool,
    /// 服务是否健康。不健康时不得假装受保护。
    healthy: bool,

    modifiers: Modifiers,
    suppressed: SuppressedKeys,

    transaction: Option<SendTransaction>,
    permit: Option<Permit>,
    snapshot: Option<VerifiedSnapshot>,
    override_approved: bool,

    current_fingerprint: ContextFingerprint,
    user_activity_generation: u64,

    next_transaction_id: u64,
    next_permit_id: u64,

    /// 回调产出的待处理请求（有界，满时丢弃最旧的诊断请求而非阻塞）。
    pending: Option<GuardRequest>,
}

impl Default for GuardCore {
    fn default() -> Self {
        Self::new(Shortcut::CtrlEnter, 1)
    }
}

impl GuardCore {
    pub fn new(shortcut: Shortcut, policy_version: u32) -> Self {
        Self {
            state: State::Disarmed,
            armed_target: None,
            shortcut,
            policy_version,
            target_foreground: false,
            ime_composing: false,
            healthy: true,
            modifiers: Modifiers::default(),
            suppressed: SuppressedKeys::default(),
            transaction: None,
            permit: None,
            snapshot: None,
            override_approved: false,
            current_fingerprint: ContextFingerprint::default(),
            user_activity_generation: 0,
            next_transaction_id: 1,
            next_permit_id: 1,
            pending: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn transaction(&self) -> Option<SendTransaction> {
        self.transaction
    }

    pub fn permit(&self) -> Option<Permit> {
        self.permit
    }

    pub fn take_request(&mut self) -> Option<GuardRequest> {
        self.pending.take()
    }

    pub fn set_shortcut(&mut self, shortcut: Shortcut) {
        self.shortcut = shortcut;
        self.revoke_permit();
    }

    pub fn set_health(&mut self, healthy: bool) {
        self.healthy = healthy;
        if !healthy {
            self.cancel(CancelReason::Shutdown);
            self.state = State::Disarmed;
        }
    }

    pub fn set_ime_composing(&mut self, composing: bool) {
        self.ime_composing = composing;
    }

    /// 目标窗口成为前台且受支持。
    pub fn arm(&mut self, target: TargetInstance, fingerprint: ContextFingerprint) {
        let same_target = self.armed_target == Some(target);
        self.armed_target = Some(target);
        self.target_foreground = true;
        if !same_target {
            self.current_fingerprint = fingerprint;
            self.cancel(CancelReason::TargetLost);
        } else {
            self.current_fingerprint = fingerprint;
        }
        if !self.state.holds_transaction() && self.healthy {
            self.state = State::ArmedIdle;
        }
    }

    /// 目标不再前台、锁屏、睡眠或不受支持。
    pub fn disarm(&mut self, reason: CancelReason) {
        self.target_foreground = false;
        self.cancel(reason);
        self.state = State::Disarmed;
    }

    /// 上下文（会话或草稿）发生变化：撤销许可并使旧结果失效。
    pub fn context_changed(&mut self, fingerprint: ContextFingerprint) {
        if fingerprint == self.current_fingerprint {
            return;
        }
        self.current_fingerprint = fingerprint;
        self.snapshot = None;
        self.revoke_permit();
        self.override_approved = false;
        if self.state.holds_transaction() {
            self.cancel(CancelReason::ContextChanged);
        }
    }

    pub fn begin_capture(&mut self) {
        if self.state == State::Held {
            self.state = State::Capturing;
        }
    }

    pub fn begin_analysis(&mut self) {
        if self.state == State::Capturing {
            self.state = State::Analyzing;
        }
    }

    /// 分析判定为低风险，进入复核。
    pub fn begin_revalidation(&mut self) {
        if self.state == State::Analyzing {
            self.state = State::Revalidating;
        }
    }

    /// 分析判定存在风险，等待用户决定。
    pub fn require_review(&mut self) {
        if self.state == State::Analyzing {
            self.state = State::AwaitingReview;
        }
    }

    /// 捕获、OCR、模型失败或超时。不得伪装为安全。
    pub fn mark_unavailable(&mut self) {
        if self.state.holds_transaction() {
            self.state = State::Unavailable;
            self.revoke_permit();
        }
    }

    /// 用户在风险窗口点击“仍要发送”，或在未完成检测时选择“本次跳过检测”。
    /// 只记录批准，不立即签发许可。
    pub fn approve_override(&mut self) {
        if matches!(self.state, State::AwaitingReview | State::Unavailable) {
            self.override_approved = true;
            self.state = State::RevalidatingOverride;
        }
    }

    pub fn override_approved(&self) -> bool {
        self.override_approved
    }

    /// 发布后台复核得到的新鲜快照。
    pub fn publish_snapshot(&mut self, snapshot: VerifiedSnapshot) {
        self.snapshot = Some(snapshot);
    }

    /// 复核一致后签发一次性许可。
    ///
    /// 返回 `None` 表示当前状态不允许签发（例如上下文已变化）。
    pub fn issue_permit(&mut self, reason: PermitReason, now: u64) -> Option<Permit> {
        if !matches!(
            self.state,
            State::Revalidating | State::RevalidatingOverride
        ) {
            return None;
        }
        let target = self.armed_target?;
        let tx = self.transaction?;
        if tx.target != target || tx.fingerprint != self.current_fingerprint {
            self.cancel(CancelReason::ContextChanged);
            return None;
        }
        if reason != PermitReason::LowRisk && !self.override_approved {
            return None;
        }
        let permit = Permit {
            permit_id: self.next_permit_id,
            target,
            fingerprint: self.current_fingerprint,
            shortcut: self.shortcut,
            policy_version: self.policy_version,
            issued_at: now,
            reason,
        };
        self.next_permit_id += 1;
        self.permit = Some(permit);
        self.state = State::PermitReady;
        Some(permit)
    }

    pub fn revoke_permit(&mut self) {
        self.permit = None;
    }

    /// 用户取消（返回修改 / 取消发送）。消息留在输入框，软件不发送任何东西。
    pub fn cancel(&mut self, reason: CancelReason) -> Option<CancelReason> {
        if !self.state.holds_transaction() && self.transaction.is_none() {
            return None;
        }
        self.transaction = None;
        self.permit = None;
        self.snapshot = None;
        self.override_approved = false;
        self.state = if self.target_foreground && self.healthy {
            State::ArmedIdle
        } else {
            State::Disarmed
        };
        Some(reason)
    }

    /// 钩子快路径。必须 O(1)、不分配、不加锁。
    pub fn on_keyboard_event(&mut self, event: KeyEvent, now: u64) -> Decision {
        let is_send_key = self.shortcut.matches(event.key, self.modifiers);

        // 修饰键状态必须始终跟踪，包括透传的事件。
        if !is_send_key {
            self.update_modifiers(event);
        }

        // 其他进程注入的事件不属于用户物理意图：不抑制、不消费许可、不计活动。
        if event.injected {
            return Decision::Pass;
        }

        // 已抑制 keydown 的配对 keyup / repeat 必须继续抑制，避免孤立事件，
        // 即使此间前台已经变化。
        if let KeyAction::Up = event.action {
            if self.suppressed.remove(event.key) {
                return Decision::Suppress;
            }
        }

        if !is_send_key {
            if matches!(event.action, KeyAction::Down { .. }) {
                self.note_user_activity();
            }
            return Decision::Pass;
        }

        // 发送键命中，但当前不具备拦截资格：透传。
        if !self.is_eligible() {
            return Decision::Pass;
        }

        match event.action {
            KeyAction::Down { .. } => self.on_send_key_down(event.key, now),
            KeyAction::Up => {
                // 未登记的 keyup（例如按下时尚未 armed）不应被吞掉。
                Decision::Pass
            }
        }
    }

    fn update_modifiers(&mut self, event: KeyEvent) {
        match event.action {
            KeyAction::Down { .. } => self.modifiers.apply(event.key.vk, true),
            KeyAction::Up => self.modifiers.apply(event.key.vk, false),
        }
    }

    fn note_user_activity(&mut self) {
        self.user_activity_generation += 1;
        // 用户继续编辑：任何既有许可与分析结果立即失效。
        if self.permit.is_some() || self.snapshot.is_some() {
            self.permit = None;
            self.snapshot = None;
            self.override_approved = false;
            self.pending = Some(GuardRequest::UserActivity {
                generation: self.user_activity_generation,
            });
        }
    }

    fn is_eligible(&self) -> bool {
        self.healthy
            && self.target_foreground
            && self.armed_target.is_some()
            && !self.ime_composing
            && self.state != State::Disarmed
    }

    fn on_send_key_down(&mut self, key: PhysicalKey, now: u64) -> Decision {
        if self.state == State::PermitReady {
            return self.try_consume_permit(key, now);
        }

        if self.state.holds_transaction() {
            // 已有活动事务：抑制新一轮发送键，不排队、不创建第二事务。
            self.suppressed.insert(key);
            return Decision::Suppress;
        }

        debug_assert_eq!(self.state, State::ArmedIdle);
        let Some(target) = self.armed_target else {
            return Decision::Pass;
        };
        let tx = SendTransaction {
            transaction_id: self.next_transaction_id,
            target,
            fingerprint: self.current_fingerprint,
            user_activity_generation: self.user_activity_generation,
            shortcut: self.shortcut,
            created_at: now,
        };
        self.next_transaction_id += 1;
        self.transaction = Some(tx);
        self.state = State::Held;
        self.suppressed.insert(key);
        self.pending = Some(GuardRequest::StartTransaction {
            transaction_id: tx.transaction_id,
        });
        Decision::Suppress
    }

    fn try_consume_permit(&mut self, key: PhysicalKey, now: u64) -> Decision {
        let Some(permit) = self.permit else {
            self.suppressed.insert(key);
            return Decision::Suppress;
        };

        let target_ok = self.armed_target == Some(permit.target);
        let fingerprint_ok = self.current_fingerprint == permit.fingerprint;
        let shortcut_ok = permit.shortcut == self.shortcut;
        let policy_ok = permit.policy_version == self.policy_version;
        let not_expired = now <= permit.expires_at();
        let snapshot_ok = self
            .snapshot
            .map(|s| s.fingerprint == permit.fingerprint && s.is_fresh_at(now))
            .unwrap_or(false);

        if target_ok && fingerprint_ok && shortcut_ok && policy_ok && not_expired && snapshot_ok {
            self.permit = None;
            self.snapshot = None;
            self.transaction = None;
            self.override_approved = false;
            self.state = State::ArmedIdle;
            self.pending = Some(GuardRequest::PermitConsumed {
                permit_id: permit.permit_id,
            });
            // 透传该物理按键 —— 发送动作由用户自己完成，软件从不合成输入。
            return Decision::ConsumePermit;
        }

        // 许可过期或快照陈旧：抑制本次按键，撤销许可，要求重新复核后再按一次。
        // 绝不异步补发刚被抑制的事件。
        self.permit = None;
        self.snapshot = None;
        self.state = State::Revalidating;
        self.suppressed.insert(key);
        self.pending = Some(GuardRequest::RecheckRequired {
            permit_id: permit.permit_id,
        });
        Decision::Suppress
    }

    /// 诊断用：是否仍有未配对的抑制按键。
    pub fn has_pending_suppressed(&self) -> bool {
        !self.suppressed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::keys::{PhysicalKey, VK_RETURN};

    const ENTER: PhysicalKey = PhysicalKey::new(VK_RETURN, 0x1C, false);
    const NUMPAD: PhysicalKey = PhysicalKey::new(VK_RETURN, 0x1C, true);
    const KEY_A: PhysicalKey = PhysicalKey::new(0x41, 0x1E, false);

    const T0: TargetInstance = TargetInstance {
        root_hwnd: 0x1234,
        pid: 42,
        process_start_time: 999,
    };
    const T1: TargetInstance = TargetInstance {
        root_hwnd: 0x5678,
        pid: 43,
        process_start_time: 1000,
    };

    fn fp(ctx: u64, scene: u64, draft: u64) -> ContextFingerprint {
        ContextFingerprint {
            context_generation: ctx,
            scene_fingerprint: scene,
            draft_fingerprint: draft,
        }
    }

    fn down(key: PhysicalKey) -> KeyEvent {
        KeyEvent {
            key,
            action: KeyAction::Down { repeat: false },
            injected: false,
        }
    }

    fn repeat(key: PhysicalKey) -> KeyEvent {
        KeyEvent {
            key,
            action: KeyAction::Down { repeat: true },
            injected: false,
        }
    }

    fn up(key: PhysicalKey) -> KeyEvent {
        KeyEvent {
            key,
            action: KeyAction::Up,
            injected: false,
        }
    }

    fn armed_guard() -> GuardCore {
        let mut g = GuardCore::new(Shortcut::Enter, 1);
        g.arm(T0, fp(1, 10, 100));
        g
    }

    /// 走完一次完整分析并签发低风险许可。
    fn issue_low_risk(g: &mut GuardCore, now: u64) -> Permit {
        g.begin_capture();
        g.begin_analysis();
        g.begin_revalidation();
        g.publish_snapshot(VerifiedSnapshot {
            fingerprint: fp(1, 10, 100),
            captured_at: now,
        });
        g.issue_permit(PermitReason::LowRisk, now).expect("permit")
    }

    #[test]
    fn disarmed_guard_passes_everything() {
        let mut g = GuardCore::new(Shortcut::Enter, 1);
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Pass);
        assert_eq!(g.on_keyboard_event(up(ENTER), 0), Decision::Pass);
        assert_eq!(g.state(), State::Disarmed);
    }

    #[test]
    fn non_target_keys_always_pass() {
        let mut g = armed_guard();
        assert_eq!(g.on_keyboard_event(down(KEY_A), 0), Decision::Pass);
        assert_eq!(g.on_keyboard_event(up(KEY_A), 0), Decision::Pass);
        assert_eq!(g.state(), State::ArmedIdle);
    }

    #[test]
    fn first_send_key_creates_transaction_and_suppresses() {
        let mut g = armed_guard();
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Suppress);
        assert_eq!(g.state(), State::Held);
        assert!(g.transaction().is_some());
        assert_eq!(
            g.take_request(),
            Some(GuardRequest::StartTransaction { transaction_id: 1 })
        );
    }

    #[test]
    fn repeat_and_paired_keyup_are_suppressed() {
        let mut g = armed_guard();
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(repeat(ENTER), 1), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(repeat(ENTER), 2), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(up(ENTER), 3), Decision::Suppress);
        assert!(!g.has_pending_suppressed());
    }

    #[test]
    fn paired_keyup_suppressed_even_after_foreground_change() {
        let mut g = armed_guard();
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Suppress);
        g.disarm(CancelReason::TargetLost);
        // 前台已变，但 keydown 已被吞，keyup 仍须抑制，避免目标收到孤立事件。
        assert_eq!(g.on_keyboard_event(up(ENTER), 1), Decision::Suppress);
        assert!(!g.has_pending_suppressed());
    }

    #[test]
    fn second_transaction_never_created_in_non_idle_states() {
        for state_setup in [
            State::Held,
            State::Capturing,
            State::Analyzing,
            State::Revalidating,
            State::AwaitingReview,
            State::Unavailable,
        ] {
            let mut g = armed_guard();
            assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Suppress);
            let first_tx = g.transaction().unwrap().transaction_id;
            assert_eq!(g.on_keyboard_event(up(ENTER), 1), Decision::Suppress);

            match state_setup {
                State::Held => {}
                State::Capturing => g.begin_capture(),
                State::Analyzing => {
                    g.begin_capture();
                    g.begin_analysis();
                }
                State::Revalidating => {
                    g.begin_capture();
                    g.begin_analysis();
                    g.begin_revalidation();
                }
                State::AwaitingReview => {
                    g.begin_capture();
                    g.begin_analysis();
                    g.require_review();
                }
                State::Unavailable => {
                    g.begin_capture();
                    g.mark_unavailable();
                }
                _ => unreachable!(),
            }
            assert_eq!(g.state(), state_setup);

            // 新一轮发送键：全部抑制，事务 ID 不变。
            assert_eq!(
                g.on_keyboard_event(down(ENTER), 10),
                Decision::Suppress,
                "state {state_setup:?}"
            );
            assert_eq!(g.on_keyboard_event(repeat(ENTER), 11), Decision::Suppress);
            assert_eq!(g.on_keyboard_event(up(ENTER), 12), Decision::Suppress);
            assert_eq!(g.transaction().unwrap().transaction_id, first_tx);
            assert!(!g.has_pending_suppressed());
        }
    }

    #[test]
    fn rapid_double_press_does_not_leak_second_send() {
        let mut g = armed_guard();
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(up(ENTER), 1), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(down(ENTER), 2), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(up(ENTER), 3), Decision::Suppress);
        assert_eq!(g.state(), State::Held);
        assert!(!g.has_pending_suppressed());
    }

    #[test]
    fn permit_consumed_once_by_physical_key() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        let permit = issue_low_risk(&mut g, 100);
        assert_eq!(g.state(), State::PermitReady);

        assert_eq!(
            g.on_keyboard_event(down(ENTER), 150),
            Decision::ConsumePermit
        );
        assert_eq!(
            g.take_request(),
            Some(GuardRequest::PermitConsumed {
                permit_id: permit.permit_id
            })
        );
        assert_eq!(g.state(), State::ArmedIdle);
        assert!(g.permit().is_none());

        // 第二次按键没有许可可用，重新进入检测。
        assert_eq!(g.on_keyboard_event(down(ENTER), 160), Decision::Suppress);
        assert_eq!(g.state(), State::Held);
    }

    #[test]
    fn permit_rejected_when_snapshot_stale() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        let permit = issue_low_risk(&mut g, 100);

        let stale = 100 + SNAPSHOT_FRESHNESS_NANOS + 1;
        assert_eq!(g.on_keyboard_event(down(ENTER), stale), Decision::Suppress);
        assert_eq!(
            g.take_request(),
            Some(GuardRequest::RecheckRequired {
                permit_id: permit.permit_id
            })
        );
        assert!(g.permit().is_none());
        assert_eq!(g.state(), State::Revalidating);
    }

    #[test]
    fn permit_rejected_after_max_lifetime() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        issue_low_risk(&mut g, 100);
        // 刷新快照，让唯一的失败原因是许可自身过期。
        let late = 100 + PERMIT_MAX_LIFETIME_NANOS + 1;
        g.publish_snapshot(VerifiedSnapshot {
            fingerprint: fp(1, 10, 100),
            captured_at: late,
        });
        assert_eq!(g.on_keyboard_event(down(ENTER), late), Decision::Suppress);
        assert!(g.permit().is_none());
    }

    #[test]
    fn permit_rejected_after_context_change() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        issue_low_risk(&mut g, 100);
        g.context_changed(fp(2, 11, 101));
        assert!(g.permit().is_none());
        assert_eq!(g.on_keyboard_event(down(ENTER), 150), Decision::Suppress);
    }

    #[test]
    fn permit_rejected_after_target_switch() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        issue_low_risk(&mut g, 100);
        g.arm(T1, fp(1, 10, 100));
        assert_eq!(g.state(), State::ArmedIdle);
        assert!(g.permit().is_none());
    }

    #[test]
    fn typing_during_analysis_invalidates_permit() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        issue_low_risk(&mut g, 100);
        assert_eq!(g.on_keyboard_event(down(KEY_A), 110), Decision::Pass);
        assert!(g.permit().is_none());
        assert!(matches!(
            g.take_request(),
            Some(GuardRequest::UserActivity { .. })
        ));
    }

    #[test]
    fn override_requires_approval_before_permit() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        g.begin_capture();
        g.begin_analysis();
        g.require_review();
        assert_eq!(g.state(), State::AwaitingReview);

        // 未批准时无法签发覆盖许可。
        assert!(g.issue_permit(PermitReason::RiskOverride, 100).is_none());

        g.approve_override();
        assert_eq!(g.state(), State::RevalidatingOverride);
        g.publish_snapshot(VerifiedSnapshot {
            fingerprint: fp(1, 10, 100),
            captured_at: 100,
        });
        let permit = g.issue_permit(PermitReason::RiskOverride, 100).unwrap();
        assert_eq!(permit.reason, PermitReason::RiskOverride);
        assert_eq!(
            g.on_keyboard_event(down(ENTER), 120),
            Decision::ConsumePermit
        );
    }

    #[test]
    fn unavailable_skip_is_distinct_reason() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        g.begin_capture();
        g.mark_unavailable();
        g.approve_override();
        g.publish_snapshot(VerifiedSnapshot {
            fingerprint: fp(1, 10, 100),
            captured_at: 50,
        });
        let permit = g.issue_permit(PermitReason::UnavailableSkip, 50).unwrap();
        assert_eq!(permit.reason, PermitReason::UnavailableSkip);
    }

    #[test]
    fn unavailable_never_yields_low_risk_permit() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        g.begin_capture();
        g.mark_unavailable();
        assert!(g.issue_permit(PermitReason::LowRisk, 50).is_none());
        assert!(g.permit().is_none());
    }

    #[test]
    fn ime_composition_always_passes_send_key() {
        let mut g = armed_guard();
        g.set_ime_composing(true);
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Pass);
        assert_eq!(g.state(), State::ArmedIdle);
        assert!(g.transaction().is_none());
    }

    #[test]
    fn injected_events_are_never_suppressed_or_consume_permit() {
        let mut g = armed_guard();
        let injected = KeyEvent {
            key: ENTER,
            action: KeyAction::Down { repeat: false },
            injected: true,
        };
        assert_eq!(g.on_keyboard_event(injected, 0), Decision::Pass);
        assert_eq!(g.state(), State::ArmedIdle);

        g.on_keyboard_event(down(ENTER), 1);
        g.on_keyboard_event(up(ENTER), 2);
        issue_low_risk(&mut g, 100);
        assert_eq!(g.on_keyboard_event(injected, 110), Decision::Pass);
        assert!(g.permit().is_some(), "注入事件不得消费用户的一次性许可");
    }

    #[test]
    fn unhealthy_guard_stops_protecting_instead_of_pretending() {
        let mut g = armed_guard();
        g.set_health(false);
        assert_eq!(g.state(), State::Disarmed);
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Pass);
    }

    #[test]
    fn numpad_enter_is_separate_physical_key() {
        let mut g = GuardCore::new(Shortcut::NumpadEnter, 1);
        g.arm(T0, fp(1, 10, 100));
        assert_eq!(g.on_keyboard_event(down(ENTER), 0), Decision::Pass);
        assert_eq!(g.on_keyboard_event(down(NUMPAD), 1), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(up(NUMPAD), 2), Decision::Suppress);
        assert_eq!(g.on_keyboard_event(up(ENTER), 3), Decision::Pass);
    }

    #[test]
    fn cancel_returns_to_idle_without_sending() {
        let mut g = armed_guard();
        g.on_keyboard_event(down(ENTER), 0);
        g.on_keyboard_event(up(ENTER), 1);
        g.begin_capture();
        g.begin_analysis();
        g.require_review();
        assert_eq!(
            g.cancel(CancelReason::UserCancelled),
            Some(CancelReason::UserCancelled)
        );
        assert_eq!(g.state(), State::ArmedIdle);
        assert!(g.transaction().is_none());
        assert!(g.permit().is_none());
    }
}
