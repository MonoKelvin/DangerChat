#![cfg(windows)]
//! IT-INT-01（手动，`--ignored`）：真实键盘 + 记事本验证吞键/放行/弹窗快捷键路由。
//!
//! 依据《开发设计文档》§11.1「集成测试（手动，`--ignored`）」与 §11.2「拦截行为在记事本/VSCode 中验证」。
//! **本用例不模拟微信、不注入任何输入**——所有按键都来自你的真实键盘。
//!
//! ```text
//! cargo test -p dc-pipeline --test it_int_01_notepad -- --ignored --nocapture
//!
//! 可选环境变量：
//!   DC_IT_TARGET   目标进程名，默认 notepad.exe（也可用 Code.exe 等任意可输入程序）
//!   DC_IT_SECONDS  观察时长（秒），默认 30
//!   DC_IT_MODE     block（默认，判定桩恒判危险）| safe（恒判安全）
//! ```
//!
//! ## 人工检查清单（mode=block）
//!
//! | 操作 | 期望 |
//! |---|---|
//! | 聚焦记事本，敲几个字后按**回车** | 控制台出现「吞键」计数 +1；**记事本里不换行** |
//! | 在弹窗存续期按 **1** | 控制台打印 `Action(Allow)`；随后按回车 → 记事本**换行**（allow-once 放行一次） |
//! | 按 **2 / 3 / 0** | 分别打印 `Cancel / Edit / Snooze`；数字**不会**进入记事本 |
//! | 切到浏览器/IDE 后按回车 | 一律放行（守护挂起，控制台状态显示 suspended） |
//! | 输入法（拼音）打字 | 按回车选字时**绝不吞键**（控制台无 swallow 增量） |

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dc_pipeline::intercept::{AlertMessage, Intercept, InterceptConfig, InterceptDeps};
use dc_pipeline::verdict::Verdict;
use dc_sys::{HookAction, KeyCallback, RealSys, SysApi};

#[test]
#[ignore = "手动集成：需要真实键盘 + 记事本（见本文件头部的检查清单）"]
fn it_int_01_notepad_interception() {
    let target = std::env::var("DC_IT_TARGET").unwrap_or_else(|_| "notepad.exe".to_string());
    let seconds: u64 = std::env::var("DC_IT_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let mode = std::env::var("DC_IT_MODE").unwrap_or_else(|_| "block".to_string());
    let block_mode = mode != "safe";

    let sys = Arc::new(RealSys::new());
    let config = InterceptConfig {
        target_process: target.clone(),
        ..Default::default()
    };
    let intercept = Arc::new(Intercept::new(InterceptDeps::new(
        Arc::clone(&sys) as Arc<dyn SysApi>,
        config.clone(),
    )));

    // 观察者钩子：先安装。低级键盘钩子「后装先执行」，因此 intercept 的钩子先裁决，
    // 被吞掉的按键不会到达观察者 → 「总按键数 − 观察者计数 = 吞键数」。
    let observed = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&observed);
    let observer: KeyCallback = Box::new(move |_ev: &dc_sys::KeyEvent| {
        counter.fetch_add(1, Ordering::SeqCst);
        HookAction::Pass
    });
    let observer_hook = sys.install_keyboard_hook(observer).expect("观察者钩子安装");

    let guard = intercept.start().expect("intercept 钩子安装");

    // 判定桩：每 200ms 用当前草稿纪元刷新一份判定，模拟完整流水线的产出（§2.3 双速循环）
    let stop = Arc::new(AtomicBool::new(false));
    let publisher = {
        let intercept = Arc::clone(&intercept);
        let stop = Arc::clone(&stop);
        let base = Instant::now();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let epoch = intercept.tracker().epoch();
                let verdict = if block_mode {
                    Verdict::block("IT-INT-01 判定桩：模拟与当前场景不匹配的消息")
                } else {
                    Verdict::safe()
                };
                intercept.slot().store(
                    verdict.with_epoch(epoch),
                    base.elapsed().as_millis() as u64 + 1,
                );
                std::thread::sleep(Duration::from_millis(200));
            }
        })
    };

    println!("\n===== IT-INT-01 手动集成验证 =====");
    println!("目标程序：{target}（请确保它正在前台且已获得焦点）");
    println!("判定模式：{mode}    观察时长：{seconds}s");
    println!("检查清单见 crates/dc-pipeline/tests/it_int_01_notepad.rs 头部注释。\n");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut shown = 0u64;
    let mut actions = 0u64;
    let mut last_state = intercept.state();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        while let Some(msg) = intercept.alerts().try_recv() {
            match msg {
                AlertMessage::Show(v) => {
                    shown += 1;
                    println!(
                        "  [弹窗请求 #{shown}] level={} epoch={} reasons={:?}",
                        v.level.as_str(),
                        v.draft_epoch,
                        v.reasons
                    );
                }
                AlertMessage::Action(action) => {
                    println!("  [弹窗动作] {action:?}（此处模拟 dc-alert 回执）");
                    intercept.apply_alert_action(action);
                    actions += 1;
                }
            }
        }
        let state = intercept.state();
        if state != last_state {
            println!("  [状态] {last_state:?} → {state:?}");
            last_state = state;
        }
    }

    stop.store(true, Ordering::SeqCst);
    let _ = publisher.join();
    drop(guard);
    drop(observer_hook);

    let total = intercept.metrics().invocations;
    let passed = observed.load(Ordering::SeqCst);
    let swallowed = total.saturating_sub(passed);
    let epoch = intercept.tracker().epoch();

    println!("\n===== 汇总 =====");
    println!("按键总数（intercept 观察到）：{total}");
    println!("放行按键（未被吞掉）：{passed}");
    println!("吞掉按键：{swallowed}");
    println!("弹窗请求：{shown}    弹窗动作：{actions}    草稿纪元：{epoch}");
    println!(
        "钩子回调平均耗时：{}ns",
        intercept.metrics().avg_duration_us() * 1000
    );
    println!("======================================\n");

    assert!(
        total > 0,
        "未观察到任何按键。请确认：① 目标程序（{target}）在前台且已获得焦点；② 在观察窗口内按过键。"
    );
    println!("IT-INT-01 采集完成：请对照文件头部的检查清单确认表格中的每一项。");
}
