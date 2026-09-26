//! 消息源模块（设计文档 §5.3）：全局键盘钩子的裁决逻辑 + 草稿纪元机制的唯一实现者。
//!
//! ## 职责边界
//!
//! 本模块**只做判定**：读取原子量、读判定槽位、决定放行还是吞键，并把结果投递到两个有界队列。
//! 它不做截图、不做 OCR、不做推理——那些在 pipeline-worker 上（§2.1 原则 1「按键路径零等待」）。
//!
//! ## 时间
//!
//! 所有时间敏感判定（TTL / allow-once / 前台去抖）都经 [`Clock`] 注入，
//! 因此测试可以确定性地快进时间，不需要 `sleep`。
//!
//! ## 与文档的一处配置扩充
//!
//! §5.3 内部结构说「3s 去抖」但配置项清单里没有对应键，这里补 `guard.foreground_debounce_ms`
//! （默认 1000），使去抖窗口可测、可调。

pub mod bus;
pub mod keys;
pub mod state;
pub mod tracker;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dc_core::{
    ConfigField, ConfigSnapshot, ConfigType, ConfigValue, MetricsRecorder, ModuleMetrics,
};
use dc_sys::{
    ForegroundInfo, HookAction, HookGuard, KeyCallback, KeyEvent, SysApi, SysError, WatchGuard,
};

use crate::clock::{Clock, MonoClock};
use crate::contract::Module;
use crate::verdict::{Verdict, VerdictSlot};

pub use bus::{AlertBus, AlertBusError, AlertMessage, Trigger, TriggerBus};
pub use keys::{alert_shortcut, is_content_key, is_send_key, AlertAction, SendKey};
pub use state::{transition, GuardEvent, GuardState};
pub use tracker::{DraftTracker, ForegroundTracker};

/// 钩子回调超时阈值（§5.3：debug 检测 > 5ms 即告警）
const HOOK_TIMEOUT: Duration = Duration::from_millis(5);

/// 从配置快照解析出的、钩子路径上只读的常量（构造后不再变化，避免回调里查配置表）。
#[derive(Debug, Clone, PartialEq)]
pub struct InterceptConfig {
    pub enabled: bool,
    pub send_key: SendKey,
    pub target_process: String,
    pub verdict_ttl: Duration,
    pub foreground_debounce: Duration,
}

impl Default for InterceptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            send_key: SendKey::Enter,
            target_process: "WeChat.exe".to_string(),
            verdict_ttl: Duration::from_millis(2_000),
            foreground_debounce: Duration::from_millis(1000),
        }
    }
}

impl InterceptConfig {
    /// 从配置快照读取（键缺失/类型不符时回退默认值，运行期绝不 panic）。
    pub fn from_snapshot(snapshot: &ConfigSnapshot) -> Self {
        let default = InterceptConfig::default();
        Self {
            enabled: snapshot.bool_or("guard.enabled", default.enabled),
            send_key: SendKey::parse(snapshot.str_or("guard.send_key", default.send_key.as_str())),
            target_process: snapshot
                .str_or("target.process_name", &default.target_process)
                .to_string(),
            verdict_ttl: Duration::from_millis(snapshot.u64_or(
                "guard.verdict_ttl_ms",
                default.verdict_ttl.as_millis() as u64,
            )),
            foreground_debounce: Duration::from_millis(snapshot.u64_or(
                "guard.foreground_debounce_ms",
                default.foreground_debounce.as_millis() as u64,
            )),
        }
    }
}

/// 模块配置 schema（FR-SYS-01：模块只声明自己的键）。
pub fn config_schema() -> Vec<ConfigField> {
    let default = InterceptConfig::default();
    vec![
        ConfigField::new(
            "guard.enabled",
            ConfigType::Bool,
            ConfigValue::Bool(default.enabled),
            "启用守护",
            "guard",
            "intercept",
        )
        .with_help("关闭后等同未安装本软件（钩子放行一切）"),
        ConfigField::new(
            "guard.send_key",
            ConfigType::Enum {
                options: SendKey::options(),
            },
            ConfigValue::Str(default.send_key.as_str().to_string()),
            "发送快捷键",
            "guard",
            "intercept",
        )
        .with_help("需与微信「发送消息」设置一致，默认 Enter"),
        ConfigField::new(
            "target.process_name",
            ConfigType::Text { max_len: 128 },
            ConfigValue::Str(default.target_process.clone()),
            "目标程序进程名",
            "target",
            "intercept",
        )
        .with_help("v1.0 默认微信；可用 notepad.exe / Code.exe 做等价验证"),
        ConfigField::new(
            "guard.verdict_ttl_ms",
            ConfigType::Int {
                min: 500,
                max: 60_000,
            },
            ConfigValue::Int(default.verdict_ttl.as_millis() as i64),
            "判定有效期（毫秒）",
            "guard",
            "intercept",
        )
        .with_help("超过该时长未续期的判定视为过时，放行"),
        ConfigField::new(
            "guard.foreground_debounce_ms",
            ConfigType::Int {
                min: 0,
                max: 60_000,
            },
            ConfigValue::Int(default.foreground_debounce.as_millis() as i64),
            "前后台切换防抖（毫秒）",
            "guard",
            "intercept",
        )
        .with_help("目标切走后多久才判定为挂起，防止 Alt+Tab 抖动导致模型反复装卸"),
    ]
}

/// 拦截模块构造参数。
pub struct InterceptDeps {
    pub sys: Arc<dyn SysApi>,
    pub clock: Arc<dyn Clock>,
    pub config: InterceptConfig,
    /// trigger 有界队列容量（满则丢弃）。
    pub trigger_capacity: usize,
    /// alert 有界队列容量（满则 fail-open）。
    pub alert_capacity: usize,
}

impl InterceptDeps {
    pub fn new(sys: Arc<dyn SysApi>, config: InterceptConfig) -> Self {
        Self {
            sys,
            clock: Arc::new(MonoClock::new()),
            config,
            trigger_capacity: 8,
            alert_capacity: 4,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_capacities(mut self, trigger: usize, alert: usize) -> Self {
        self.trigger_capacity = trigger;
        self.alert_capacity = alert;
        self
    }
}

/// 消息源模块。
pub struct Intercept {
    sys: Arc<dyn SysApi>,
    clock: Arc<dyn Clock>,
    config: InterceptConfig,
    /// 目标进程名（热更新；on_foreground 的 sys-thread 读，低频，锁可接受）。
    target: std::sync::RwLock<String>,
    /// 发送键（热更新；钩子裁决路径读，原子解码保持 O(1)）。
    send_key: AtomicU8,
    /// 守护开关（热更新；refresh_state 读）。
    enabled: AtomicBool,
    /// 判定有效期毫秒（热更新；回车裁决读）。
    verdict_ttl_ms: AtomicU64,
    state: AtomicU8,
    foreground: ForegroundTracker,
    tracker: Arc<DraftTracker>,
    slot: Arc<VerdictSlot>,
    triggers: Arc<TriggerBus>,
    alerts: Arc<AlertBus>,
    metrics: MetricsRecorder,
    log_dir: std::path::PathBuf,
}

impl Intercept {
    pub fn new(deps: InterceptDeps) -> Self {
        let InterceptDeps {
            sys,
            clock,
            config,
            trigger_capacity,
            alert_capacity,
        } = deps;
        let foreground = ForegroundTracker::new(config.foreground_debounce.as_millis() as u64);
        let tracker = DraftTracker::new();
        if config.target_process.trim().is_empty() {
            // §5.3 错误矩阵：目标进程名配置为空必须在装配期被拒绝（调用方检查 config）
            tracing::error!("target.process_name 为空：守护不会生效");
        }
        let send_key = AtomicU8::new(SendKey::encode(config.send_key));
        let target = std::sync::RwLock::new(config.target_process.clone());
        let enabled = AtomicBool::new(config.enabled);
        let verdict_ttl_ms = AtomicU64::new(config.verdict_ttl.as_millis() as u64);
        Self {
            sys,
            clock,
            config,
            target,
            send_key,
            enabled,
            verdict_ttl_ms,
            state: AtomicU8::new(GuardState::Suspended.to_u8()),
            foreground,
            tracker: Arc::new(tracker),
            slot: Arc::new(VerdictSlot::new()),
            triggers: Arc::new(TriggerBus::new(trigger_capacity)),
            alerts: Arc::new(AlertBus::new(alert_capacity)),
            metrics: MetricsRecorder::default(),
            log_dir: std::path::PathBuf::new(),
        }
    }

    /// 热更新目标进程名（set_config → 即时生效，无需重启）。
    pub fn set_target_process(&self, name: &str) {
        if let Ok(mut t) = self.target.write() {
            *t = name.trim().to_string();
        }
    }

    /// 热更新发送键。
    pub fn set_send_key(&self, key: SendKey) {
        self.send_key.store(SendKey::encode(key), Ordering::SeqCst);
    }

    /// 热更新守护开关。
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }

    /// 热更新判定有效期。
    pub fn set_verdict_ttl_ms(&self, ms: u64) {
        self.verdict_ttl_ms
            .store(ms.clamp(100, 60_000), Ordering::SeqCst);
    }

    /// 热更新前后台切换防抖窗口（`guard.foreground_debounce_ms`）。
    ///
    /// 与其余 guard.* 键一致走即时生效：去抖窗口只在 `note()` 排定挂起计划时取用，
    /// 因此改配置立即影响下一次「目标离开前台」的判定。
    pub fn set_foreground_debounce_ms(&self, ms: u64) {
        self.foreground.set_debounce_ms(ms);
    }

    fn current_send_key(&self) -> SendKey {
        SendKey::decode(self.send_key.load(Ordering::SeqCst))
    }

    /// 便捷构造：默认时钟 + 传入的 sys。
    pub fn with_sys(sys: Arc<dyn SysApi>, config: InterceptConfig) -> Self {
        Self::new(InterceptDeps::new(sys, config))
    }

    pub fn config(&self) -> &InterceptConfig {
        &self.config
    }

    pub fn log_dir(&self) -> &std::path::Path {
        &self.log_dir
    }

    pub fn clock_arc(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock)
    }

    pub fn slot_arc(&self) -> Arc<VerdictSlot> {
        Arc::clone(&self.slot)
    }

    pub fn triggers_arc(&self) -> Arc<TriggerBus> {
        Arc::clone(&self.triggers)
    }

    pub fn tracker_arc(&self) -> Arc<DraftTracker> {
        Arc::clone(&self.tracker)
    }

    pub fn slot(&self) -> Arc<VerdictSlot> {
        Arc::clone(&self.slot)
    }

    pub fn tracker(&self) -> &DraftTracker {
        &self.tracker
    }

    pub fn triggers(&self) -> &TriggerBus {
        &self.triggers
    }

    pub fn alerts_arc(&self) -> Arc<AlertBus> {
        Arc::clone(&self.alerts)
    }

    pub fn alerts(&self) -> &AlertBus {
        &self.alerts
    }

    pub fn metrics(&self) -> ModuleMetrics {
        self.metrics.snapshot()
    }

    /// 当前守护状态（顺带让到期的挂起去抖落地）。
    pub fn state(&self) -> GuardState {
        let now = self.clock.now_ms();
        self.refresh_state(now)
    }

    /// 目标程序是否处于前台（含去抖判定）。
    pub fn target_active(&self) -> bool {
        self.foreground.evaluate(self.clock.now_ms())
    }

    // ---------------------------------------------------------------
    // 钩子回调（§5.3 伪码的忠实实现）
    // ---------------------------------------------------------------

    /// 按键裁决。**必须在 < 1ms 内返回**：只有原子读、时间比较与 `try_send`。
    pub fn on_key(&self, ev: &KeyEvent) -> HookAction {
        let started = Instant::now();
        let action = self.decide(ev);
        let elapsed = started.elapsed();
        self.metrics.record(elapsed);
        if elapsed >= HOOK_TIMEOUT {
            tracing::warn!(
                elapsed_us = elapsed.as_micros() as u64,
                vk = ev.vk,
                "钩子回调超时（> 5ms），请检查是否有同步 IO 混入回调路径"
            );
            self.metrics.record_failure();
        }
        action
    }

    fn decide(&self, ev: &KeyEvent) -> HookAction {
        let now = self.clock.now_ms();
        let state = self.refresh_state(now);

        // 1) 挂起 / 暂停：放行一切（§2.4）
        if state.passes_everything() {
            return HookAction::Pass;
        }

        // 2) 输入法组合中：绝不吞键（FR-SRC-09）。
        //
        // 组合期间草稿**同样在变**（候选词正在输入框里成形），因此必须推进纪元
        // ——但**不触发 Pipeline**（候选框会进入 msg_input ROI 造成 OCR 噪声/误判），
        // 等组合结束后用户按回车时才触发快环。
        //
        // 注意：组合态优先于 Cooldown 分支，因此组合期间数字键 1/2/3 会被输入法吃掉、
        // 不会路由为弹窗动作——此时用鼠标点弹窗按钮即可（弹窗本身不夺焦点）。
        if self.sys.ime_composing() {
            if ev.is_key_down && (is_send_key(ev, self.current_send_key()) || ev.is_ime_consumed())
            {
                // 组合期间**不触发Pipeline**：输入法候选框会进入 msg_input ROI，
                // 造成 OCR 噪声和误判。只推进纪元，等组合结束后用户按回车时才触发。
                self.tracker.bump();
            }
            return HookAction::Pass;
        }

        // 3) 弹窗存续期：数字键代转动作；发送键吞掉但不重复弹窗；其余放行
        if state == GuardState::Cooldown {
            if !ev.is_key_down {
                return HookAction::Pass;
            }
            if let Some(action) = alert_shortcut(ev) {
                if self.alerts.send_action(action).is_err() {
                    tracing::debug!(
                        action = action.as_str(),
                        "弹窗动作入队失败（用户仍可点鼠标）"
                    );
                }
                return HookAction::Swallow;
            }
            if is_send_key(ev, self.current_send_key()) {
                return HookAction::Swallow;
            }
            // §2.4：弹窗存续期「用户可继续打字，纪元正常推进」——必须推进，
            // 否则用户改稿后旧判定仍与纪元匹配，弹窗关掉就会对**已经改过的草稿**下手
            // （此处按 §2.4 状态机语义执行，§5.3 伪码在该分支漏了这一步）。
            if is_content_key(ev.vk) {
                self.tracker.bump();
                let _ = self.triggers.offer_fast();
            }
            return HookAction::Pass;
        }

        // 4) Active：发送键裁决 / 内容键推进纪元
        if is_send_key(ev, self.current_send_key()) {
            if !ev.is_key_down {
                return HookAction::Pass;
            }
            if self.tracker.is_snoozed() {
                return HookAction::Pass;
            }
            // 严格 fail-open：判定未就绪、纪元不匹配或 TTL 过期时立即放行，
            // 同时异步触��发当前草稿分析。发送钩子不等待，也不因缺少判定而吞键。
            let epoch_now = self.tracker.epoch();
            let fresh = self.slot.load_fresh(
                Duration::from_millis(self.verdict_ttl_ms.load(Ordering::SeqCst)),
                now,
            );
            let verdict = match fresh {
                Some(v) if v.draft_epoch == epoch_now => v,
                _ => {
                    let _ = self.triggers.offer_fast();
                    tracing::debug!(
                        epoch = epoch_now,
                        "fail-open：判定未就绪，放行并异步触发分析"
                    );
                    return HookAction::Pass;
                }
            };
            // 只有 Block（「已阻断发送」）才吞键拦截；Warn（「发送提醒」）与 Safe 一样放行，
            // 不吞键、不弹窗（用户要求：提醒级别不打扰，仅阻断级别拦下）。
            if verdict.level == crate::verdict::VerdictLevel::Block {
                if self.alerts.request_show(Arc::clone(&verdict)).is_ok() {
                    self.transition(GuardEvent::AlertShown, now);
                    tracing::info!(
                        level = verdict.level.as_str(),
                        score = verdict.score,
                        "拦截发送键：已入队弹窗"
                    );
                    HookAction::Swallow
                } else {
                    // 弹窗通道异常 → fail-open，绝不静默吞键（§2.2）
                    tracing::error!("弹窗入队失败：改判放行（fail-open）");
                    HookAction::Pass
                }
            } else {
                HookAction::Pass
            }
        } else {
            if ev.is_key_down && is_content_key(ev.vk) {
                self.tracker.bump();
                let _ = self.triggers.offer_fast();
            }
            HookAction::Pass
        }
    }

    /// 前台切换事件（由 dc-sys 的 `SetWinEventHook` 回调在 sys-thread 上调用）。
    pub fn on_foreground(&self, info: ForegroundInfo) {
        let now = self.clock.now_ms();
        let is_target = self.matches_target(&info);
        // 只记录**与守护目标相关**的切换：切入目标 / 从目标切出。
        // 非目标程序间切换（切浏览器→切编辑器、鼠标聚焦等）不入日志，避免刷屏。
        let was_target = self.foreground.is_target_foreground();
        self.foreground.note(is_target, now);
        if is_target {
            // 唤醒即时生效：用户切回目标程序后不该还要等去抖窗口
            self.transition(GuardEvent::ForegroundActive, now);
        }
        if is_target && !was_target {
            tracing::info!(process = %info.process_name, state = self.state().as_str(), "切入守护目标");
        } else if !is_target && was_target {
            tracing::info!(process = %info.process_name, state = self.state().as_str(), "切出守护目标");
        }
    }

    /// 前台进程是否为目标程序（仅比较进程名，不做任何进程内部探测，C-05/C-06）。
    pub fn matches_target(&self, info: &ForegroundInfo) -> bool {
        let target = self
            .target
            .read()
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        !target.is_empty() && info.process_name.eq_ignore_ascii_case(&target)
    }

    /// worker 分析完成回调（在 pipeline-worker 线程调用）。
    ///
    /// 只记下待发纪元。worker 算完在此裁决：
    /// - 纪元匹配 + 安全 → 清零待发标志（判定已入槽，用户再按回车即可命中放行）；
    /// - 纪元匹配 + 危险 → 主动入队弹窗并进入 Cooldown（消息从未发送出去）；
    /// - 纪元不匹配（草稿又改了）→ 不动待发标志，等对应纪元的判定或下次回车重新裁决。
    ///

    /// 应用弹窗动作（dc-alert → dc-bridge → 这里）。
    pub fn apply_alert_action(&self, action: AlertAction) {
        let now = self.clock.now_ms();
        match action {
            AlertAction::Cancel => {
                self.transition(GuardEvent::AlertResolved, now);
                // 关闭≠改稿：内容没变（纪元没变），刷新判定新鲜度让 TTL 重新计时，
                // 下次回车仍命中同一危险判定→再次弹窗（符合「关闭=下次还拦」）。
                self.slot.refresh(now);
            }
            AlertAction::Snooze => {
                self.tracker.snooze_current();
                self.transition(GuardEvent::AlertResolved, now);
                tracing::info!(
                    epoch = self.tracker.epoch(),
                    "用户选择「本次不再提示」：静默当前纪元，改稿即恢复"
                );
            }
        }
    }

    /// 暂停 / 恢复守护（FR-SRC-07）。
    pub fn set_paused(&self, paused: bool) {
        let now = self.clock.now_ms();
        if paused {
            self.transition(GuardEvent::Pause, now);
        } else {
            let target_active = self.foreground.evaluate(now);
            self.transition(GuardEvent::Resume { target_active }, now);
        }
    }

    // ---------------------------------------------------------------
    // 装配
    // ---------------------------------------------------------------

    /// 安装全局键盘钩子 + 前台事件监听。返回的守卫 `Drop` 即卸载。
    pub fn start(self: &Arc<Self>) -> Result<InterceptGuard, SysError> {
        let key_intercept = Arc::clone(self);
        let callback: KeyCallback = Box::new(move |ev: &KeyEvent| key_intercept.on_key(ev));
        let hook = self.sys.install_keyboard_hook(callback)?;

        let fg_intercept = Arc::clone(self);
        let watch = self.sys.watch_foreground(Box::new(move |info| {
            fg_intercept.on_foreground(info);
        }))?;

        // 用当前前台窗口初始化（避免启动瞬间误判为挂起）
        if let Some(info) = self.sys.foreground() {
            self.on_foreground(info);
        }

        Ok(InterceptGuard {
            _hook: hook,
            _watch: watch,
        })
    }

    // ---------------------------------------------------------------
    // 内部
    // ---------------------------------------------------------------

    /// 把前台派生态（Active/Suspended）落到存储里，并返回当前有效状态。
    fn refresh_state(&self, now_ms: u64) -> GuardState {
        if !self.enabled.load(Ordering::SeqCst) {
            return GuardState::Paused;
        }
        let stored = GuardState::from_u8(self.state.load(Ordering::SeqCst));
        let active = self.foreground.evaluate(now_ms);
        let derived = match stored {
            // 显式态不被前台事件派生
            GuardState::Paused | GuardState::Cooldown => stored,
            GuardState::Active if !active => GuardState::Suspended,
            GuardState::Suspended if active => GuardState::Active,
            other => other,
        };
        if derived != stored {
            self.state.store(derived.to_u8(), Ordering::SeqCst);
            tracing::debug!(
                from = stored.as_str(),
                to = derived.as_str(),
                "守护状态迁移"
            );
        }
        derived
    }

    fn transition(&self, event: GuardEvent, _now_ms: u64) {
        let from = GuardState::from_u8(self.state.load(Ordering::SeqCst));
        let to = state::transition(from, event);
        if to != from {
            self.state.store(to.to_u8(), Ordering::SeqCst);
            tracing::info!(from = from.as_str(), to = to.as_str(), "守护状态迁移");
        }
    }
}

impl Module for Intercept {
    fn id(&self) -> &'static str {
        "intercept"
    }

    fn config_schema(&self) -> Vec<ConfigField> {
        config_schema()
    }

    fn init(&mut self, ctx: &dc_core::ModuleContext) -> Result<(), dc_core::ModuleError> {
        self.log_dir = ctx.log_dir.clone();
        // 配置热更新：本模块的常量在装配期快照，改动需重建实例（§4 ConfigSnapshot 语义）
        tracing::debug!(
            target = %self.config.target_process,
            send_key = self.config.send_key.as_str(),
            "intercept 初始化"
        );
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), dc_core::ModuleError> {
        Ok(())
    }

    fn metrics(&self) -> ModuleMetrics {
        self.metrics.snapshot()
    }
}

/// 钩子与前台监听的守卫，`Drop` 即卸载（避免测试/热重载后残留全局钩子）。
pub struct InterceptGuard {
    _hook: HookGuard,
    _watch: WatchGuard,
}

impl std::fmt::Debug for InterceptGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InterceptGuard")
    }
}

/// 空判定槽位辅助：`Verdict` 的常用构造（供测试与上层复用）。
pub fn stale_verdict_slot() -> Arc<VerdictSlot> {
    Arc::new(VerdictSlot::new())
}

/// 便于上层的默认构造（生产装配：真实 sys + 单调时钟）。
pub fn default_clock() -> Arc<dyn Clock> {
    Arc::new(MonoClock::new())
}

/// 供 `guard` 编排器在 Stage 失败时清槽（§5.8 fail-open）。
pub fn clear_on_failure(slot: &VerdictSlot, reason: &str) {
    slot.clear();
    tracing::debug!(reason, "判定槽位已清空（fail-open）");
}

/// 判定与纪元的组合工具：把新判定绑定到当前草稿纪元（§5.8）。
pub fn bind_epoch(verdict: Verdict, epoch: u64) -> Verdict {
    verdict.with_epoch(epoch)
}
