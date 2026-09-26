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
//!   DC_IT_TARGET    目标进程名，默认 notepad.exe（也可用 Code.exe 等任意可输入程序）
//!   DC_IT_SECONDS   观察时长（秒），默认 30
//!   DC_IT_MODE      block（默认，判定桩恒判危险）| safe（恒判安全）
//!   DC_IT_PUBLISH   loop（默认，每 200ms 刷新判定）| once（只发布一次，用于验证 TTL 过期 fail-open）
//!   DC_IT_TRACE     1 = 在钩子回调内逐键打印放行事件（仅排障用，会增加回调耗时）
//! ```
//!
//! ## 观测原理
//!
//! 键盘回调按注册顺序分发、任一 `Swallow` 即终止（见 `SysApi` 契约）。因此本用例把
//! **观察者注册在 intercept 之后**：
//!
//! ```text
//! 每个按键 → [intercept 回调] 放行? → [观察者回调] 计数
//!                  │
//!                  └─ 吞掉 → 观察者收不到 → 差值即吞键数
//! ```
//!
//! ## 人工检查清单（mode=block，请逐项核对）
//!
//! | # | 操作 | 期望 |
//! |---|---|---|
//! | 1 | 聚焦记事本，敲几个字后按**回车** | `[tick]`「吞掉」+1；**记事本里不换行** |
//! | 2 | 弹窗存续期按 **1** | 出现 `[弹窗动作] Allow`；随后按回车 → 记事本**换行**（allow-once 放行一次） |
//! | 3 | 弹窗存续期按 **2** / **3** / **0** | 分别打印 `Cancel / Edit / Snooze`；数字**不会**进入记事本 |
//! | 4 | 切到浏览器/IDE 后按回车，再切回记事本 | `state=suspended` 期间一律放行；切回即时 `active` |
//! | 5 | 用拼音打字（`输入法键` 持续增长），组合中按回车选字 | `吞掉` **不增加**；`输入法键` 增长；记事本正常出字 |
//! | 6 | `DC_IT_PUBLISH=once` 重跑：打完字后静置 > 2s 再按回车 | `吞掉` 不增��加，判定超 TTL 后严格 fail-open 放行 |
//!
//! 第 6 项验证严格 fail-open：判定缺失或过期时不阻塞发送。

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
    let publish_once = std::env::var("DC_IT_PUBLISH").is_ok_and(|v| v == "once");
    let trace = std::env::var("DC_IT_TRACE").is_ok_and(|v| v == "1");

    let sys = Arc::new(RealSys::new());
    let config = InterceptConfig {
        target_process: target.clone(),
        ..Default::default()
    };
    let intercept = Arc::new(Intercept::new(InterceptDeps::new(
        Arc::clone(&sys) as Arc<dyn SysApi>,
        config.clone(),
    )));

    let guard = intercept.start().expect("intercept 钩子安装");

    // 观察者注册在 intercept **之后**：只有被放行的键才会到达这里（见文件头「观测原理」）
    let observed = Arc::new(AtomicU64::new(0));
    let ime_keys = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&observed);
    let ime_counter = Arc::clone(&ime_keys);
    let observer: KeyCallback = Box::new(move |ev: &dc_sys::KeyEvent| {
        counter.fetch_add(1, Ordering::SeqCst);
        if ev.vk == dc_sys::VK_PROCESSKEY && ev.is_key_down {
            // 被输入法消费的键：能到达观察者，说明组合期间确实没有被吞（FR-SRC-09）
            ime_counter.fetch_add(1, Ordering::SeqCst);
        }
        if trace {
            println!("    [放行] vk={:#04X} down={}", ev.vk, ev.is_key_down);
        }
        HookAction::Pass
    });
    let observer_hook = sys.install_keyboard_hook(observer).expect("观察者注册");

    // 判定桩：默认每 200ms 用当前草稿纪元刷新一份判定，模拟完整流水线的产出（§2.3 双速循环）。
    // `DC_IT_PUBLISH=once` 时只发布一次，用于观察判定超 TTL 后的严格 fail-open 放行。
    let stop = Arc::new(AtomicBool::new(false));
    let publisher = {
        let intercept = Arc::clone(&intercept);
        let stop = Arc::clone(&stop);
        let base = Instant::now();
        std::thread::spawn(move || {
            let mut published = false;
            while !stop.load(Ordering::SeqCst) {
                if !publish_once || !published {
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
                    published = true;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        })
    };

    println!("\n===== IT-INT-01 手动集成验证 =====");
    println!("目标程序：{target}（请确保它正在前台且已获得焦点）");
    println!("判定模式：{mode}    观察时长：{seconds}s    逐键 trace：{trace}");
    if publish_once {
        println!("判定发布：once（只发布一次；约 2s 后判定超过 TTL → 之后的发送键应全部放行）");
    } else {
        println!("判定发布：loop（每 200ms 刷新，判定始终新鲜）");
    }
    println!("检查清单见 crates/dc-pipeline/tests/it_int_01_notepad.rs 头部注释。");
    println!(
        "提示：**弹窗 UI 属于 M6（dc-alert）**，当前里程碑只有后端拦截链路，弹窗请求打印在下面。\n"
    );

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut shown = 0u64;
    let mut actions = 0u64;
    let mut last_state = intercept.state();
    let mut last_tick = Instant::now();
    let mut cooldown_hinted = false;

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
            if state == dc_pipeline::GuardState::Cooldown && !cooldown_hinted {
                println!(
                    "        （存续期：1=仍然发送 / 2=取消 / 3=返回编辑 / 0=本次不再提示；发送键持续被吞）"
                );
                cooldown_hinted = true;
            }
            last_state = state;
        }

        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
            let total = intercept.metrics().invocations;
            let passed = observed.load(Ordering::SeqCst);
            println!(
                "  [tick] state={:?} epoch={} 按键={total} 放行={passed} 吞掉={} 输入法键={} ime_composing={}",
                state,
                intercept.tracker().epoch(),
                total.saturating_sub(passed),
                ime_keys.load(Ordering::SeqCst),
                sys.ime_composing()
            );
        }
    }

    stop.store(true, Ordering::SeqCst);
    let _ = publisher.join();
    drop(guard);
    drop(observer_hook);

    let total = intercept.metrics().invocations;
    let passed = observed.load(Ordering::SeqCst);
    let swallowed = total.saturating_sub(passed);
    let metrics = intercept.metrics();

    println!("\n===== 汇总 =====");
    println!("按键总数（intercept 观察到）：{total}");
    println!("放行按键（未被吞掉）：{passed}");
    println!("吞掉按键：{swallowed}");
    println!(
        "弹窗请求：{shown}    弹窗动作：{actions}    草稿纪元：{}",
        intercept.tracker().epoch()
    );
    println!(
        "钩子回调耗时：avg={}ns max={}ns（metrics 以纳秒累计）",
        metrics.avg_duration_ns(),
        metrics.max_duration_ns
    );
    println!("======================================\n");

    assert!(
        total > 0,
        "未观察到任何按键。请确认：① 目标程序（{target}）在前台且已获得焦点；② 在观察窗口内按过键。"
    );
    println!("IT-INT-01 采集完成：请对照文件头部的检查清单确认每一行。");
}
