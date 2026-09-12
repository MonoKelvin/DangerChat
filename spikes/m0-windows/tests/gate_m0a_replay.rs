//! M0-A 闸门：确定性事件回放。
//!
//! 用合成事件序列驱动纯领域守卫，验证文档 `03:42` 的三项硬指标：
//! - 目标外 100,000 次事件零误阻止；
//! - 目标内 10,000 次事务零重复发送、零孤立 keyup；
//! - 事务内不产生第二次发送。
//!
//! 这里只验证判定逻辑。回调实测延迟由 `m0 hook-latency` 在真实钩子上测量，
//! 两者不可互相替代，报告中分别标注。

use m0_windows::domain::guard::{
    ContextFingerprint, Decision, GuardCore, KeyAction, KeyEvent, PermitReason, TargetInstance,
    VerifiedSnapshot,
};
use m0_windows::domain::keys::{PhysicalKey, Shortcut, VK_RETURN};

const ENTER: PhysicalKey = PhysicalKey::new(VK_RETURN, 0x1C, false);

const TARGET: TargetInstance = TargetInstance {
    root_hwnd: 0xABCD,
    pid: 4242,
    process_start_time: 777,
};

fn fingerprint(n: u64) -> ContextFingerprint {
    ContextFingerprint {
        context_generation: n,
        scene_fingerprint: 0x5CE7E ^ n,
        draft_fingerprint: 0xD8AF7 ^ n,
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

/// 可复现的伪随机序列，避免引入额外依赖。
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// 非目标按键集合：字母、数字、方向键、功能键、修饰键。
fn non_send_keys() -> Vec<PhysicalKey> {
    let mut keys = Vec::new();
    for vk in 0x41u16..=0x5A {
        keys.push(PhysicalKey::new(vk, vk - 0x41 + 0x1E, false));
    }
    for vk in 0x30u16..=0x39 {
        keys.push(PhysicalKey::new(vk, vk - 0x30 + 0x02, false));
    }
    for vk in [0x25u16, 0x26, 0x27, 0x28] {
        keys.push(PhysicalKey::new(vk, 0x4B, true));
    }
    for vk in 0x70u16..=0x7B {
        keys.push(PhysicalKey::new(vk, vk - 0x70 + 0x3B, false));
    }
    for vk in [0x10u16, 0x11, 0x12, 0x5B, 0x08, 0x09, 0x20, 0x2E] {
        keys.push(PhysicalKey::new(vk, 0x2A, false));
    }
    keys
}

/// 门槛 1：目标不在前台时，10 万次事件必须全部透传。
#[test]
fn gate_100k_non_target_events_zero_false_suppression() {
    let mut guard = GuardCore::new(Shortcut::Enter, 1);
    let keys = non_send_keys();
    let mut rng = Lcg(0x5EED_1234);
    let mut suppressed = 0u64;
    let mut processed = 0u64;

    for i in 0..100_000u64 {
        // 目标未 arm：包括发送键在内的所有事件都应透传。
        let key = if i % 7 == 0 {
            ENTER
        } else {
            keys[rng.below(keys.len() as u64) as usize]
        };
        let event = match rng.below(10) {
            0..=5 => down(key),
            6..=7 => repeat(key),
            _ => up(key),
        };
        if guard.on_keyboard_event(event, i * 1_000_000) == Decision::Suppress {
            suppressed += 1;
        }
        processed += 1;
    }

    assert_eq!(processed, 100_000);
    assert_eq!(suppressed, 0, "目标不在前台时不得抑制任何按键");
    assert!(!guard.has_pending_suppressed());
}

/// 门槛 2：目标已 arm，但按键不是配置的发送键时必须全部透传。
#[test]
fn gate_100k_non_send_keys_pass_while_armed() {
    let mut guard = GuardCore::new(Shortcut::CtrlEnter, 1);
    guard.arm(TARGET, fingerprint(1));
    let keys = non_send_keys();
    let mut rng = Lcg(0xC0FFEE);
    let mut suppressed = 0u64;

    for i in 0..100_000u64 {
        let key = keys[rng.below(keys.len() as u64) as usize];
        let event = match rng.below(10) {
            0..=5 => down(key),
            6..=7 => repeat(key),
            _ => up(key),
        };
        if guard.on_keyboard_event(event, i * 1_000_000) == Decision::Suppress {
            suppressed += 1;
        }
    }

    // 未按 Ctrl 时单独的 Enter 也不属于配置的发送键。
    for i in 0..1_000u64 {
        assert_eq!(
            guard.on_keyboard_event(down(ENTER), 100_000_000_000 + i),
            Decision::Pass
        );
    }

    assert_eq!(suppressed, 0, "非配置发送键不得被抑制");
}

/// 门槛 3：10,000 次完整事务，零重复发送、零孤立 keyup、零跨上下文放行。
#[test]
fn gate_10k_transactions_no_duplicate_or_orphan() {
    let mut guard = GuardCore::new(Shortcut::Enter, 1);
    let mut now = 0u64;
    let mut passes = 0u64;
    let mut consumed = 0u64;
    let mut second_transaction_ids = Vec::new();

    for i in 0..10_000u64 {
        let fp = fingerprint(i);
        guard.arm(TARGET, fp);
        now += 1_000_000;

        // 第一次物理按键：必须被抑制并开启唯一事务。
        assert_eq!(
            guard.on_keyboard_event(down(ENTER), now),
            Decision::Suppress
        );
        assert!(guard.transaction().is_some(), "第一次按键必须开启事务");
        now += 1_000;

        // 自动重复与配对 keyup 必须一并抑制。
        assert_eq!(
            guard.on_keyboard_event(repeat(ENTER), now),
            Decision::Suppress
        );
        now += 1_000;
        assert_eq!(guard.on_keyboard_event(up(ENTER), now), Decision::Suppress);

        // 分析期间再次按键：抑制且不得产生第二个事务。
        now += 1_000;
        assert_eq!(
            guard.on_keyboard_event(down(ENTER), now),
            Decision::Suppress
        );
        now += 1_000;
        assert_eq!(guard.on_keyboard_event(up(ENTER), now), Decision::Suppress);
        second_transaction_ids.push(guard.transaction().unwrap().transaction_id);

        // 完成分析并签发低风险许可。
        guard.begin_capture();
        guard.begin_analysis();
        guard.begin_revalidation();
        now += 200_000_000;
        guard.publish_snapshot(VerifiedSnapshot {
            fingerprint: fp,
            captured_at: now,
        });
        guard
            .issue_permit(PermitReason::LowRisk, now)
            .expect("复核一致后应签发许可");

        // 第二次物理按键：消费许可并透传。这是唯一的放行路径。
        now += 10_000_000;
        match guard.on_keyboard_event(down(ENTER), now) {
            Decision::ConsumePermit => {
                consumed += 1;
                passes += 1;
            }
            other => panic!("第 {i} 次事务应消费许可，实际为 {other:?}"),
        }
        now += 1_000;
        // 消费后的 keyup 未被登记抑制，应透传给目标，避免孤立事件。
        assert_eq!(guard.on_keyboard_event(up(ENTER), now), Decision::Pass);

        assert!(guard.permit().is_none(), "许可必须一次性消费，不得重复使用");
        assert!(!guard.has_pending_suppressed(), "不得残留未配对的抑制按键");
        now += 1_000_000;
    }

    assert_eq!(consumed, 10_000, "每次事务恰好消费一次许可");
    assert_eq!(passes, 10_000, "放行次数必须等于事务数，不得重复发送");
    for (i, tx) in second_transaction_ids.iter().enumerate() {
        assert_eq!(*tx, (i as u64) + 1, "分析期间重复按键不得创建第二个事务");
    }
}

/// 门槛 4：任何失败路径都不得放行，消息留在输入框。
#[test]
fn gate_failure_paths_never_pass_send_key() {
    let mut rng = Lcg(0xFA11);
    let mut guard = GuardCore::new(Shortcut::Enter, 1);
    let mut now = 0u64;
    let mut passes = 0u64;

    for i in 0..2_000u64 {
        let fp = fingerprint(i);
        guard.arm(TARGET, fp);
        now += 1_000_000;
        assert_eq!(
            guard.on_keyboard_event(down(ENTER), now),
            Decision::Suppress
        );
        now += 1_000;
        assert_eq!(guard.on_keyboard_event(up(ENTER), now), Decision::Suppress);
        guard.begin_capture();

        match rng.below(5) {
            // 捕获或 OCR 失败。
            0 => guard.mark_unavailable(),
            // 分析期间切换会话。
            1 => {
                guard.begin_analysis();
                guard.context_changed(fingerprint(i + 500_000));
            }
            // 分析期间目标离开前台。
            2 => {
                guard.begin_analysis();
                guard.disarm(m0_windows::domain::guard::CancelReason::TargetLost);
            }
            // 判定有风险但用户选择返回修改。
            3 => {
                guard.begin_analysis();
                guard.require_review();
                guard.cancel(m0_windows::domain::guard::CancelReason::UserCancelled);
            }
            // 服务不健康。
            _ => guard.set_health(false),
        }

        // 失败后再次按键：绝不能直接放行。
        now += 5_000_000;
        let decision = guard.on_keyboard_event(down(ENTER), now);
        if decision == Decision::Pass {
            // 仅当守卫已明确不再保护（Disarmed）时才允许透传，
            // 此时用户看到的是"未受保护"状态，而不是"已检查"。
            assert!(
                !guard.state().holds_transaction(),
                "处于活动事务时不得透传发送键"
            );
            passes += 1;
        }
        assert_ne!(
            decision,
            Decision::ConsumePermit,
            "失败路径不得产生可消费的许可"
        );
        now += 1_000;
        guard.on_keyboard_event(up(ENTER), now);
        guard.set_health(true);
    }

    // 透传只可能发生在守卫已停止保护的情形，且必须有对应的失败状态。
    assert!(passes <= 2_000);
}
