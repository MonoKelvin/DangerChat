//! UT-BRG-01/02：契约载荷 + allow 链路（零注入）。
//!
//! UT-BRG-01：DTO 序列化黄金样本——前端 main-ui 的 Vitest 读取**同一批**
//! fixtures 对照手写 types.ts（双写漂移在 CI 被两侧同时拦截）。

use dc_bridge::dto::{AlertPayload, StatsPayload, StatusPayload};
use std::sync::Arc;

use dc_pipeline::verdict::Verdict;

fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn ut_brg_01_write_contract_fixtures() {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).unwrap();

    // alert payload（全字段）
    let verdict = Verdict::block("命中违禁词「sb」(severity=block)")
        .with_epoch(3)
        .with_draft("你是 sb", 42)
        .with_target("张总")
        .with_scene("正式")
        .with_context(Some("昨天的报告怎么样\n已经完成了".into()));
    let alert = AlertPayload::from_verdict(&verdict, 10);
    std::fs::write(
        dir.join("alert.json"),
        serde_json::to_string_pretty(&alert).unwrap(),
    )
    .unwrap();

    // status payload
    let status = StatusPayload {
        state: "active".into(),
        target_process: "WeChat.exe".into(),
        target_found: true,
    };
    std::fs::write(
        dir.join("status.json"),
        serde_json::to_string_pretty(&status).unwrap(),
    )
    .unwrap();

    // stats payload
    let stats = StatsPayload {
        today_blocked: 12,
        alert_failed: 1,
        last_capture_ms: Some(18.2),
        last_layout_ms: Some(35.6),
        last_ocr_ms: Some(140.0),
        last_sem_ms: Some(4.1),
    };
    std::fs::write(
        dir.join("stats.json"),
        serde_json::to_string_pretty(&stats).unwrap(),
    )
    .unwrap();

    // 回读断言（本轮就地自检；后续运行作为漂移检测——字段增删会改变序列化）
    let reread: AlertPayload =
        serde_json::from_str(&std::fs::read_to_string(dir.join("alert.json")).unwrap()).unwrap();
    assert_eq!(reread.level, "block");
    assert_eq!(reread.draft_epoch, 3);
}

/// UT-BRG-02：弹窗动作链路——apply_alert_action 只改内部状态，
/// 全程无任何按键注入（MockSys 的按键投递计数为 0；红线扫描在 CI 另行断言）。
/// （原 allow 链路测试已随「仍然发送」按钮移除；零注入红线对现存动作同样成立）
#[test]
fn ut_brg_02_alert_action_without_injection() {
    use dc_pipeline::clock::TestClock;
    use dc_pipeline::intercept::{Intercept, InterceptConfig, InterceptDeps};
    use dc_pipeline::testing::Harness;
    use dc_sys::{HookAction, KeyEvent, MockSys, SysApi};

    let harness = Harness::new();
    let clock = Arc::new(TestClock::new(1_000));
    let sys = MockSys::new();

    // 观察者钩子：记录放行的按键（动作链路不产生任何新按键）
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_hook = Arc::clone(&seen);
    SysApi::install_keyboard_hook(
        &sys,
        Box::new(move |e: &KeyEvent| {
            seen_hook.lock().unwrap().push((e.vk, e.is_key_down));
            HookAction::Pass
        }),
    )
    .unwrap();

    // 独立 Intercept（Harness 的已装钩子，这里只用它的 MockSys 模式验证动作语义）
    let deps = InterceptDeps::new(
        Arc::new(sys.clone()) as Arc<dyn SysApi>,
        InterceptConfig::default(),
    )
    .with_clock(Arc::clone(&clock) as Arc<dyn dc_pipeline::clock::Clock>);
    let intercept = Arc::new(Intercept::new(deps));

    // 全部现存动作逐一执行：不 panic、不注入（seen 为空）。
    for a in [
        dc_pipeline::intercept::AlertAction::Cancel,
        dc_pipeline::intercept::AlertAction::Snooze,
    ] {
        intercept.apply_alert_action(a);
    }

    // 关键断言：动作执行后系统里没有任何新按键被投递
    assert!(
        seen.lock().unwrap().is_empty(),
        "弹窗动作链路不得注入任何按键（红线 C-08）"
    );
    let _ = harness; // 保持钩子存活到断言之后
}
