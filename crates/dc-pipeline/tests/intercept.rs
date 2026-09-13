//! UT-INT-01 ~ UT-INT-10：消息源模块（设计文档 §5.3 测试矩阵）
//!
//! 全部基于 `Harness`（MockSys + TestClock），不依赖真实桌面、不做任何 `sleep`：
//! 时间由 `TestClock` 手动推进，TTL / allow-once / 去抖都能确定性验证。

use std::time::{Duration, Instant};

use dc_core::{ConfigCenter, ConfigType, ConfigValue};
use dc_pipeline::intercept::{
    state, transition, AlertMessage, GuardEvent, GuardState, Intercept, InterceptConfig, SendKey,
};
use dc_pipeline::testing::Harness;
use dc_pipeline::{AlertAction, Verdict};
use dc_sys::{HookAction, KeyEvent, MockSys, SysApi};

const VK_A: u16 = 0x41;
const VK_BACK: u16 = 0x08;
const VK_DELETE: u16 = 0x2E;
const VK_V: u16 = 0x56;
const VK_LEFT: u16 = 0x25;
const VK_F1: u16 = 0x70;
const VK_F12: u16 = 0x7B;
const VK_ENTER: u16 = 0x0D;

/// UT-INT-01 状态机全迁移表（Active/Suspended/Paused/Cooldown × 事件）
///
/// 表里写的是**规格**（每个格子手写期望值），不是实现的重述。
#[test]
fn ut_int_01_state_machine_full_transition_table() {
    use GuardEvent as E;
    use GuardState as S;

    const EVENTS: [E; 8] = [
        E::ForegroundActive,
        E::ForegroundInactive,
        E::Pause,
        E::Resume {
            target_active: true,
        },
        E::Resume {
            target_active: false,
        },
        E::AlertShown,
        E::AlertResolved,
        E::AlertTimeout,
    ];

    //                             FgActive     FgInactive   Pause        Resume(active) Resume(inactive) AlertShown  AlertResolved AlertTimeout
    let expected: [[S; 8]; 4] = [
        /* Active */
        [
            S::Active,
            S::Suspended,
            S::Paused,
            S::Active,
            S::Active,
            S::Cooldown,
            S::Active,
            S::Active,
        ],
        /* Suspended */
        [
            S::Active,
            S::Suspended,
            S::Paused,
            S::Suspended,
            S::Suspended,
            S::Suspended,
            S::Suspended,
            S::Suspended,
        ],
        /* Paused */
        [
            S::Paused,
            S::Paused,
            S::Paused,
            S::Active,
            S::Suspended,
            S::Paused,
            S::Paused,
            S::Paused,
        ],
        /* Cooldown */
        [
            S::Cooldown,
            S::Suspended,
            S::Paused,
            S::Cooldown,
            S::Cooldown,
            S::Cooldown,
            S::Active,
            S::Active,
        ],
    ];

    for (row, from) in [S::Active, S::Suspended, S::Paused, S::Cooldown]
        .into_iter()
        .enumerate()
    {
        for (col, event) in EVENTS.into_iter().enumerate() {
            assert_eq!(
                transition(from, event),
                expected[row][col],
                "迁移不符：{from:?} + {event:?}"
            );
        }
    }

    // 状态码往返（AtomicU8 存取依赖它）
    for s in [S::Active, S::Suspended, S::Paused, S::Cooldown] {
        assert_eq!(S::from_u8(s.to_u8()), s);
        assert!(s.as_str().chars().all(|c| c.is_ascii_lowercase()));
    }
    assert!(S::Suspended.passes_everything() && S::Paused.passes_everything());
    assert!(!S::Active.passes_everything() && !S::Cooldown.passes_everything());
}

/// UT-INT-01（集成侧）：弹窗关闭 / 仍然发送 / 本次不再提示 三条路径都回到 Active
#[test]
fn ut_int_01_alert_paths_return_to_active() {
    let h = Harness::new();
    h.target_foreground();
    assert_eq!(h.intercept.state(), GuardState::Active);

    // 进入 Cooldown 的三要素：纪元匹配 + 新鲜 + 危险
    let enter_cooldown = |h: &Harness| {
        let _ = h.press(VK_A);
        h.publish(Verdict::block("命中违禁词"));
        assert_eq!(h.enter(), HookAction::Swallow);
        assert_eq!(h.intercept.state(), GuardState::Cooldown);
    };

    // 取消发送
    enter_cooldown(&h);
    h.intercept.apply_alert_action(AlertAction::Cancel);
    assert_eq!(h.intercept.state(), GuardState::Active);

    // 返回编辑
    enter_cooldown(&h);
    h.intercept.apply_alert_action(AlertAction::Edit);
    assert_eq!(h.intercept.state(), GuardState::Active);

    // 仍然发送：先置 allow-once 再退出 Cooldown
    enter_cooldown(&h);
    h.intercept.apply_alert_action(AlertAction::Allow);
    assert_eq!(h.intercept.state(), GuardState::Active);
    assert!(h.intercept.tracker().allow_once_armed(h.now()));
    // 危信不代发：由用户自己再按一次回车完成发送（同时消费掉放行标志，C-08）
    assert_eq!(
        h.enter(),
        HookAction::Pass,
        "allow-once 放行用户自己按下的回车"
    );
    assert!(!h.intercept.tracker().allow_once_armed(h.now()));

    // 本次不再提示
    enter_cooldown(&h);
    h.intercept.apply_alert_action(AlertAction::Snooze);
    assert_eq!(h.intercept.state(), GuardState::Active);
    assert!(h.intercept.tracker().is_snoozed());
}

/// UT-INT-02 内容键 → draft_epoch 递增且触发 fast offer；非内容键（方向键、F1~F12）不递增
#[test]
fn ut_int_02_content_keys_bump_epoch() {
    let h = Harness::new();
    h.target_foreground();

    let mut epoch = h.intercept.tracker().epoch();
    for vk in [VK_A, 0x31, 0x20, VK_BACK, VK_DELETE, 0x6E] {
        let _ = h.press(vk);
        epoch += 1;
        assert_eq!(
            h.intercept.tracker().epoch(),
            epoch,
            "vk={vk:#X} 应推进纪元"
        );
    }
    // Ctrl+V 同样算内容修改
    let _ = h.press_ctrl(VK_V);
    epoch += 1;
    assert_eq!(h.intercept.tracker().epoch(), epoch);

    // 非内容键：方向键、Home/End、F1~F12、修饰键、Insert
    for vk in [VK_LEFT, 0x24, 0x23, VK_F1, VK_F12, 0x10, 0x11, 0x12, 0x2D] {
        let _ = h.press(vk);
        assert_eq!(
            h.intercept.tracker().epoch(),
            epoch,
            "vk={vk:#X} 不应推进纪元"
        );
    }

    // 抬键不推进纪元
    let _ = h.release(VK_A);
    assert_eq!(h.intercept.tracker().epoch(), epoch);

    // 内容键同时触发快环 offer
    assert!(h.intercept.triggers().offered() >= epoch);
}

/// UT-INT-03 纪元匹配 + 新鲜 + Block → Swallow 且 alert 入队成功、进入 Cooldown
#[test]
fn ut_int_03_block_swallows_and_enqueues_alert() {
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    let _ = h.press(0x42);
    h.publish(Verdict::block("与正式场景不匹配"));

    assert_eq!(h.enter(), HookAction::Swallow, "危险判定必须吞键");
    assert_eq!(h.intercept.state(), GuardState::Cooldown);
    assert_eq!(h.intercept.alerts().shown(), 1);
    assert_eq!(h.intercept.alerts().failed(), 0);

    let msg = h.intercept.alerts().try_recv().expect("弹窗请求已入队");
    match msg {
        AlertMessage::Show(v) => {
            assert_eq!(v.draft_epoch, h.intercept.tracker().epoch());
            assert_eq!(v.level, dc_pipeline::VerdictLevel::Block);
            assert!(v.reasons[0].contains("正式场景"));
        }
        other => panic!("应为 Show，实际 {other:?}"),
    }

    // Warn 同样拦截
    let h2 = Harness::new();
    h2.target_foreground();
    let _ = h2.press(VK_A);
    h2.publish(Verdict::warn("疑似不匹配"));
    assert_eq!(h2.enter(), HookAction::Swallow);
}

/// UT-INT-04 纪元不等 / 过期 / 无缓存 / 挂起 / 暂停 → 全部 Pass（fail-open 五分支）
#[test]
fn ut_int_04_fail_open_branches() {
    // 1) 无缓存
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    assert_eq!(h.enter(), HookAction::Pass, "无判定 → 放行");

    // 2) 纪元不等：判定基于旧草稿
    let h = Harness::new();
    h.target_foreground();
    h.publish_with_epoch(Verdict::block("旧判定"), 0);
    let _ = h.press(VK_A); // 纪元 → 1，判定仍是 0
    assert_eq!(h.enter(), HookAction::Pass, "纪元不等 → 放行");
    assert_eq!(h.intercept.state(), GuardState::Active);

    // 3) 判定过期（TTL 默认 2s）
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    h.advance(2_001);
    assert_eq!(h.enter(), HookAction::Pass, "超过 TTL → 放行");

    // 4) 挂起（目标切走并去抖确认）
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    h.other_foreground("Code.exe");
    h.advance(3_000);
    assert_eq!(h.intercept.state(), GuardState::Suspended);
    assert_eq!(h.enter(), HookAction::Pass, "挂起 → 放行一切");

    // 5) 暂停
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    h.intercept.set_paused(true);
    assert_eq!(h.intercept.state(), GuardState::Paused);
    assert_eq!(h.enter(), HookAction::Pass, "暂停 → 放行一切");
    // 恢复后重新生效（判定仍新鲜、纪元未变）
    h.intercept.set_paused(false);
    assert_eq!(h.intercept.state(), GuardState::Active);
    assert_eq!(h.enter(), HookAction::Swallow, "恢复后恢复拦截");
}

/// UT-INT-05 allow-once 标志：置位后下一次发送键放行并失效；超时后失效
#[test]
fn ut_int_05_allow_once_semantics() {
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));

    assert_eq!(h.enter(), HookAction::Swallow);
    h.intercept.apply_alert_action(AlertAction::Allow);
    assert!(h.intercept.tracker().allow_once_armed(h.now()));

    // 第二次按键：消费标志 → 放行（用户自己按下的这一次）
    assert_eq!(h.enter(), HookAction::Pass, "allow-once 放行一次");
    assert!(
        !h.intercept.tracker().allow_once_armed(h.now()),
        "放行后立即失效"
    );

    // 标志已失效：同纪元同判定仍然拦截（进入 Cooldown）
    assert_eq!(h.intercept.state(), GuardState::Active);
    assert_eq!(h.enter(), HookAction::Swallow, "标志失效后恢复拦截");
    h.intercept.apply_alert_action(AlertAction::Cancel);

    // 超时后失效
    h.intercept.apply_alert_action(AlertAction::Allow);
    h.advance(30_000);
    assert!(!h.intercept.tracker().allow_once_armed(h.now()));
    h.publish(Verdict::block("命中")); // 换一份新鲜判定，隔离「超时」这一个变量
    assert_eq!(h.enter(), HookAction::Swallow, "超时不再放行");
}

/// UT-INT-06 回调压测：10 万次 mock 按键 p99 < 1ms（CI 基准断言）
#[test]
fn ut_int_06_hook_callback_latency() {
    let h = Harness::new();
    h.target_foreground();
    h.publish(Verdict::block("压力测试用"));

    const N: usize = 100_000;
    let mut samples = Vec::with_capacity(N);
    let ev = h.key_event(VK_A, false);
    for _ in 0..N {
        let t0 = Instant::now();
        let _ = h.intercept.on_key(&ev);
        samples.push(t0.elapsed().as_nanos() as u64);
    }
    samples.sort_unstable();
    let p50 = samples[N / 2];
    let p99 = samples[N * 99 / 100];
    let max = samples[N - 1];

    println!("钩子回调耗时：p50={p50}ns p99={p99}ns max={max}ns（N={N}）");
    assert!(
        p99 < 1_000_000,
        "钩子回调 p99 超预算：p50={p50}ns p99={p99}ns max={max}ns（NFR-02 预算 < 1ms）"
    );
    assert!(p50 < 100_000, "p50 应远低于 1ms：{p50}ns");
}

/// UT-INT-07 IME 组合中：发送键放行且纪元 +1；非发送键放行
#[test]
fn ut_int_07_ime_composing_passes_and_bumps() {
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    let epoch = h.intercept.tracker().epoch();

    h.sys.set_ime_composing(true);

    // 组合中的字母（实际是 VK_PROCESSKEY，这里用普通键验证「绝不吞键」）
    assert_eq!(h.press(0x49), HookAction::Pass);
    assert_eq!(
        h.intercept.tracker().epoch(),
        epoch,
        "组合中普通键不推进纪元"
    );

    // 回车 = 选字确认 → 放行 + 纪元 +1（FR-SRC-09）
    assert_eq!(h.enter(), HookAction::Pass, "组合中回车绝不吞键");
    assert_eq!(h.intercept.tracker().epoch(), epoch + 1);
    assert_eq!(h.intercept.alerts().shown(), 0, "组合中不得弹窗");

    // 组合结束：判定纪元已落后 → 仍然 fail-open
    h.sys.set_ime_composing(false);
    assert_eq!(h.enter(), HookAction::Pass);
    h.publish(Verdict::block("命中"));
    assert_eq!(h.enter(), HookAction::Swallow, "组合结束后恢复守护");
}

/// UT-INT-08 alert 入队失败 → 放行（fail-open，绝不静默吞键）
#[test]
fn ut_int_08_alert_enqueue_failure_fails_open() {
    // alert 容量 0 = 汇合通道，无接收方等待时 try_send 立即失败
    let h = Harness::with_capacities(InterceptConfig::default(), 8, 0);
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));

    assert_eq!(
        h.enter(),
        HookAction::Pass,
        "弹窗入队失败必须放行，绝不静默吞键"
    );
    assert_eq!(h.intercept.state(), GuardState::Active, "未进入 Cooldown");
    assert_eq!(h.intercept.alerts().failed(), 1);
    assert_eq!(h.intercept.alerts().shown(), 0);

    // 对照：容量正常时同样场景会吞键（证明上一条不是空断言）
    let h2 = Harness::new();
    h2.target_foreground();
    let _ = h2.press(VK_A);
    h2.publish(Verdict::block("命中"));
    assert_eq!(h2.enter(), HookAction::Swallow);
}

/// UT-INT-09 snooze：同纪元发送键放行；纪元变化后恢复拦截
#[test]
fn ut_int_09_snooze_only_silences_current_epoch() {
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    assert_eq!(h.enter(), HookAction::Swallow);
    h.intercept.apply_alert_action(AlertAction::Snooze);
    assert_eq!(h.intercept.state(), GuardState::Active);

    // 同纪元：静默放行（判定仍然新鲜）
    assert_eq!(h.enter(), HookAction::Pass, "本次不再提示 → 放行");
    assert_eq!(h.enter(), HookAction::Pass, "仍然静默");

    // 改稿：纪元 +1 → 自动恢复守护
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    assert!(!h.intercept.tracker().is_snoozed(), "改稿即恢复");
    assert_eq!(h.enter(), HookAction::Swallow);
}

/// UT-INT-10 Cooldown 期间数字键 1/2/3 被截获路由为对应 AlertAction 且 Swallow；其余键不受影响
#[test]
fn ut_int_10_cooldown_routes_alert_shortcuts() {
    let h = Harness::new();
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));
    assert_eq!(h.enter(), HookAction::Swallow);
    let _ = h.intercept.alerts().try_recv(); // 消费 Show
    assert_eq!(h.intercept.state(), GuardState::Cooldown);

    for (vk, want) in [
        (0x31u16, AlertAction::Allow),
        (0x32, AlertAction::Cancel),
        (0x33, AlertAction::Edit),
        (0x30, AlertAction::Snooze),
    ] {
        assert_eq!(h.press(vk), HookAction::Swallow, "vk={vk:#X} 必须被截获");
        match h.intercept.alerts().try_recv() {
            Some(AlertMessage::Action(got)) => assert_eq!(got, want),
            other => panic!("vk={vk:#X} 应路由为 {want:?}，实际 {other:?}"),
        }
    }

    // 其余键放行且纪元正常推进（用户可继续编辑）
    let epoch = h.intercept.tracker().epoch();
    assert_eq!(h.press(VK_A), HookAction::Pass);
    assert_eq!(h.intercept.tracker().epoch(), epoch + 1);

    // 发送键：吞掉但不重复弹窗
    assert_eq!(h.enter(), HookAction::Swallow);
    assert_eq!(h.intercept.alerts().shown(), 1, "不得重复弹窗");
    assert!(h.intercept.alerts().try_recv().is_none());

    // 组合中的数字键不劫持（IME 优先）
    h.sys.set_ime_composing(true);
    assert_eq!(h.press(0x31), HookAction::Pass);
}

// ---------------------------------------------------------------------------
// 补充用例（不属于矩阵编号，但覆盖规格里的关键行为）
// ---------------------------------------------------------------------------

/// 前台去抖：切走后 3s 内仍算前台；3s 后挂起；切回即时唤醒
#[test]
fn foreground_debounce_and_instant_wake() {
    let h = Harness::new();
    h.target_foreground();
    assert_eq!(h.intercept.state(), GuardState::Active);

    h.other_foreground("explorer.exe");
    assert_eq!(h.intercept.state(), GuardState::Active, "去抖窗口内不挂起");
    h.advance(2_999);
    assert_eq!(h.intercept.state(), GuardState::Active);
    h.advance(1);
    assert_eq!(h.intercept.state(), GuardState::Suspended, "超过 3s 才挂起");

    h.target_foreground();
    assert_eq!(
        h.intercept.state(),
        GuardState::Active,
        "切回即时唤醒，不等去抖"
    );

    // 去抖期间切回：不应挂起
    h.other_foreground("cmd.exe");
    h.advance(1_000);
    h.target_foreground();
    h.advance(10_000);
    assert_eq!(h.intercept.state(), GuardState::Active);
}

/// 目标进程名匹配：大小写不敏感、空配置视为不匹配
#[test]
fn target_matching_semantics() {
    let h = Harness::new();
    h.sys.set_foreground(Some(dc_sys::ForegroundInfo {
        hwnd: dc_sys::Hwnd(1),
        pid: 1,
        process_name: "wechat.exe".to_string(),
    }));
    let info = h.sys.foreground().unwrap();
    assert!(h.intercept.matches_target(&info), "大小写不敏感");

    let h2 = Harness::with_config(InterceptConfig {
        target_process: String::new(),
        ..Default::default()
    });
    h2.sys.set_foreground(Some(dc_sys::ForegroundInfo {
        hwnd: dc_sys::Hwnd(1),
        pid: 1,
        process_name: "wechat.exe".to_string(),
    }));
    let info2 = h2.sys.foreground().unwrap();
    assert!(
        !h2.intercept.matches_target(&info2),
        "空目标名不匹配任何程序"
    );
    assert_eq!(h2.intercept.state(), GuardState::Suspended);
}

/// Ctrl+Enter 配置：只有带 Ctrl 的回车算发送键
#[test]
fn send_key_configuration() {
    let h = Harness::with_config(InterceptConfig {
        send_key: SendKey::CtrlEnter,
        ..Default::default()
    });
    h.target_foreground();
    let _ = h.press(VK_A);
    h.publish(Verdict::block("命中"));

    assert_eq!(
        h.press(VK_ENTER),
        HookAction::Pass,
        "纯回车是换行，必须放行"
    );
    assert_eq!(
        h.press_ctrl(VK_ENTER),
        HookAction::Swallow,
        "Ctrl+Enter 才是发送键"
    );

    // 默认配置下 Ctrl+Enter 不是发送键
    let h2 = Harness::new();
    h2.target_foreground();
    let _ = h2.press(VK_A);
    h2.publish(Verdict::block("命中"));
    assert_eq!(h2.press_ctrl(VK_ENTER), HookAction::Pass);
    assert_eq!(h2.enter(), HookAction::Swallow);
}

/// 配置 schema 与快照解析一致（防止 schema 里的键名与读取代码漂移）
#[test]
fn config_schema_matches_snapshot_parsing() {
    let dir = tempfile::TempDir::new().unwrap();
    let center = ConfigCenter::open(dir.path().join("config.toml")).unwrap();
    let schema = dc_pipeline::intercept_config_schema();
    center.register_module("intercept", schema.clone()).unwrap();
    let snapshot = center.finalize().unwrap();

    let config = InterceptConfig::from_snapshot(&snapshot);
    assert_eq!(
        config,
        InterceptConfig::default(),
        "默认快照应解析出默认配置"
    );

    // 全部键都能从 schema 找到（无悬空读取）
    for field in &schema {
        assert!(field.key.contains('.'), "配置键应为点分路径：{}", field.key);
        assert!(snapshot.contains(&field.key), "快照缺少 {}", field.key);
        assert_eq!(field.owner, "intercept");
    }
    assert_eq!(schema.len(), 8);
    assert!(schema
        .iter()
        .any(|f| matches!(f.ty, ConfigType::Enum { .. }) && f.key == "guard.send_key"));

    // 改配置 → 解析生效
    center
        .set("guard.send_key", ConfigValue::Str("ctrl+enter".into()))
        .unwrap();
    center
        .set(
            "target.process_name",
            ConfigValue::Str("notepad.exe".into()),
        )
        .unwrap();
    center
        .set("guard.verdict_ttl_ms", ConfigValue::Int(500))
        .unwrap();
    let snapshot = center.snapshot();
    let config = InterceptConfig::from_snapshot(&snapshot);
    assert_eq!(config.send_key, SendKey::CtrlEnter);
    assert_eq!(config.target_process, "notepad.exe");
    assert_eq!(config.verdict_ttl, Duration::from_millis(500));
}

/// 装配：`start()` 装钩子与前台监听，守卫 Drop 即卸载
#[test]
fn start_installs_and_drop_uninstalls() {
    let sys = MockSys::new();
    let intercept = std::sync::Arc::new(Intercept::with_sys(
        std::sync::Arc::new(sys.clone()) as std::sync::Arc<dyn SysApi>,
        InterceptConfig::default(),
    ));
    {
        let guard = intercept.start().expect("安装成功");
        assert!(sys.is_hooked());
        assert!(sys.watch_installed());
        drop(guard);
    }
    assert!(!sys.is_hooked(), "守卫释放后钩子已卸载");
    assert!(!sys.watch_installed(), "守卫释放后前台监听已取消");

    // 装钩子后按键经 dc-sys → intercept 全链路（等价 IT-INT-01 的自动化部分）
    let sys = MockSys::new();
    let intercept = std::sync::Arc::new(Intercept::with_sys(
        std::sync::Arc::new(sys.clone()) as std::sync::Arc<dyn SysApi>,
        InterceptConfig::default(),
    ));
    let _guard = intercept.start().unwrap();
    sys.emit_foreground(dc_sys::ForegroundInfo {
        hwnd: dc_sys::Hwnd(1),
        pid: 1,
        process_name: "WeChat.exe".to_string(),
    });
    let action = sys.feed_key(KeyEvent {
        vk: VK_A,
        scan_code: 0,
        is_key_down: true,
        is_injected: false,
        ctrl: false,
        alt: false,
        shift: false,
    });
    assert_eq!(action, HookAction::Pass);
    assert_eq!(intercept.tracker().epoch(), 1, "经钩子回调同样推进纪元");
}

/// 纯函数层：按键分类与状态机的边界
#[test]
fn key_classification_edges() {
    assert_eq!(
        state::transition(GuardState::Active, GuardEvent::AlertShown),
        GuardState::Cooldown
    );
    for vk in [VK_LEFT, 0x26u16, 0x27, 0x28, 0x23, 0x24, 0x2D, VK_F1] {
        assert!(
            !dc_pipeline::intercept::is_content_key(vk),
            "{vk:#X} 不是内容键"
        );
    }
    for vk in [
        VK_A, 0x30u16, 0x39, 0x6F, 0xBA, 0xDE, VK_BACK, VK_DELETE, 0x20,
    ] {
        assert!(
            dc_pipeline::intercept::is_content_key(vk),
            "{vk:#X} 是内容键"
        );
    }
}

/// 观察者模式：注册在裁决者**之后**的回调只能看到被放行的按键
/// （IT-INT-01 的观测原理；回归：dc-sys 曾用单槽回调存钩子，导致观察者永远收不到事件）
#[test]
fn observer_after_decider_sees_only_passed_keys() {
    use std::sync::atomic::AtomicU64;

    let h = Harness::new();
    h.target_foreground();
    h.publish(Verdict::block("命中"));

    let seen = std::sync::Arc::new(AtomicU64::new(0));
    let counter = std::sync::Arc::clone(&seen);
    let _observer = h
        .sys
        .install_keyboard_hook(Box::new(move |_ev: &KeyEvent| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            HookAction::Pass
        }))
        .expect("观察者注册");

    // 经 dc-sys 全链路投递：吞掉的键不会到达观察者
    let enter = h.key_event(VK_ENTER, false);
    assert_eq!(h.sys.feed_key(enter), HookAction::Swallow, "危险判定吞键");
    assert_eq!(
        seen.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "吞掉的按键不得分发到后续回调"
    );

    // 放行的键会到达观察者
    let letter = h.key_event(VK_A, false);
    assert_eq!(h.sys.feed_key(letter), HookAction::Pass);
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        h.intercept.metrics().invocations,
        2,
        "intercept 两个键都看到了"
    );
    assert!(
        h.intercept.metrics().avg_duration_ns() > 0,
        "指标应为纳秒精度"
    );
}
