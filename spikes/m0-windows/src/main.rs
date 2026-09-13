//! 危信 M0 尖峰命令行。
//!
//! 子命令：
//! - `window-probe` 打印当前前台窗口快照，验证只读窗口元数据路径。
//! - `capture-probe` 捕获前台窗口所在显示器并按 DWM 边界裁剪，报告耗时与像素统计。
//! - `hook-latency N` 安装真实低级键盘钩子 N 秒，测量回调延迟分布。该模式不 arm
//!   任何目标，因此所有按键都应透传，同时验证“非目标状态零误阻止”。
//!
//! 本程序不调用任何输入注入 API，也不访问任何第三方程序的进程、文件或窗口内容。

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("window-probe") => window_probe(),
        Some("capture-probe") => {
            let rounds: u32 = args
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20)
                .clamp(1, 500);
            capture_probe(rounds);
        }
        Some("layout-eval") => layout_eval(),
        Some("ocr-export") => ocr_export(),
        Some("e2e-latency") => {
            let rounds: usize = args
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30)
                .clamp(5, 500);
            e2e_latency(rounds);
        }
        Some("full-latency") => {
            let rounds: usize = args
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20)
                .clamp(1, 200);
            // 第二个参数是等待物理按键的预算秒数，便于在无人值守时验证失败路径。
            let budget_secs: u64 = args
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300)
                .clamp(5, 3600);
            full_latency(rounds, budget_secs);
        }
        Some("ime-probe") => ime_probe(),
        Some("ime-hook-probe") => {
            let secs: u64 = args
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30)
                .clamp(3, 600);
            ime_hook_probe(secs);
        }
        Some("hook-latency") => {
            let secs: u64 = args
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20)
                .clamp(3, 600);
            hook_latency(secs);
        }
        other => {
            eprintln!("未知子命令: {other:?}");
            eprintln!(
                "用法: m0 <window-probe|capture-probe [次数]|layout-eval|ocr-export|\
                 e2e-latency [次数]|full-latency [次数] [等待秒数]|ime-probe|\
                 ime-hook-probe [秒]|hook-latency [秒]>"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("M0 尖峰仅支持 Windows。");
    std::process::exit(2);
}

#[cfg(windows)]
fn window_probe() {
    use m0_windows::platform::window::foreground_snapshot;

    match foreground_snapshot() {
        Some(s) => {
            println!("前台窗口快照（仅操作系统窗口元数据）：");
            println!("  root_hwnd          = 0x{:X}", s.instance.root_hwnd);
            println!("  pid                = {}", s.instance.pid);
            println!("  process_start_time = {}", s.instance.process_start_time);
            println!(
                "  dwm_bounds         = ({}, {}) {}x{}",
                s.bounds.left,
                s.bounds.top,
                s.width(),
                s.height()
            );
            println!("  dpi                = {}", s.dpi);
            println!("  minimized          = {}", s.minimized);
            println!("  visible            = {}", s.visible);
        }
        None => println!("当前没有可读取的前台窗口。"),
    }
}

#[cfg(windows)]
fn capture_probe(rounds: u32) {
    use m0_windows::platform::capture::{capture_window_region, declare_dpi_awareness};
    use m0_windows::platform::clock::MonotonicClock;
    use m0_windows::platform::window::foreground_snapshot;

    declare_dpi_awareness();
    let clock = MonotonicClock::new();

    let Some(snapshot) = foreground_snapshot() else {
        eprintln!("没有前台窗口，无法捕获。");
        std::process::exit(1);
    };

    println!(
        "目标窗口：0x{:X}  pid={}  dpi={}",
        snapshot.instance.root_hwnd, snapshot.instance.pid, snapshot.dpi
    );
    println!(
        "DWM 边界：({}, {}) {}x{}",
        snapshot.bounds.left,
        snapshot.bounds.top,
        snapshot.width(),
        snapshot.height()
    );

    let mut durations = Vec::with_capacity(rounds as usize);
    let mut last_desc = String::new();

    for i in 0..rounds {
        match capture_window_region(snapshot.instance.root_hwnd, snapshot.client, &clock) {
            Ok(frame) => {
                durations.push(frame.capture_nanos);
                if i == 0 {
                    last_desc = format!(
                        "{}x{} stride={} bytes={} origin=({}, {}) monitor=({}, {})-({}, {})",
                        frame.width,
                        frame.height,
                        frame.stride,
                        frame.byte_len(),
                        frame.origin.x,
                        frame.origin.y,
                        frame.monitor_bounds.left,
                        frame.monitor_bounds.top,
                        frame.monitor_bounds.right,
                        frame.monitor_bounds.bottom
                    );
                }
            }
            Err(e) => {
                eprintln!("第 {i} 次捕获失败：{e:?}");
                std::process::exit(1);
            }
        }
    }

    durations.sort_unstable();
    let pct = |p: f64| -> u64 {
        let idx = ((durations.len() as f64 - 1.0) * p).round() as usize;
        durations[idx]
    };

    println!();
    println!("=== M0-C 捕获与裁剪 ===");
    println!("帧            = {last_desc}");
    println!("样本数        = {}", durations.len());
    println!("p50           = {:.1} ms", pct(0.50) as f64 / 1e6);
    println!("p95           = {:.1} ms", pct(0.95) as f64 / 1e6);
    println!(
        "最大          = {:.1} ms",
        durations[durations.len() - 1] as f64 / 1e6
    );
    println!();

    let p95_ok = pct(0.95) <= 80_000_000;
    println!("门槛 捕获裁剪 p95 ≤ 80 ms : {}", verdict(p95_ok));
    println!("注：本次为 GDI 降级路径实测，DXGI/WGC 路径尚未接入。");
    if !p95_ok {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn layout_eval() {
    use m0_windows::harness::layout_eval::run_matrix;

    println!("=== M0-C 区域定位评测（自有测试窗口，无真实聊天内容）===");
    let results = run_matrix();

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut errors = 0usize;
    let mut min_title = f32::MAX;
    let mut min_compose = f32::MAX;
    let mut min_context = f32::MAX;
    let mut capture_times = Vec::new();
    let mut detect_times = Vec::new();
    let mut signatures = std::collections::BTreeMap::new();

    for result in &results {
        match result {
            Ok(case) => {
                let ok = case.passed();
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
                min_title = min_title.min(case.title_iou);
                min_compose = min_compose.min(case.compose_iou);
                min_context = min_context.min(case.context_iou);
                capture_times.push(case.capture_nanos);
                detect_times.push(case.detect_nanos);
                *signatures.entry(case.layout_signature).or_insert(0usize) += 1;

                println!(
                    "  [{}] {:<22} {:>5}x{:<5} dpi={:<4} title={:.3} ctx={:.3} compose={:.3} conf={:.2}",
                    if ok { "PASS" } else { "FAIL" },
                    format!("{} {}", case.theme, case.label),
                    case.client_width,
                    case.client_height,
                    case.dpi,
                    case.title_iou,
                    case.context_iou,
                    case.compose_iou,
                    case.confidence
                );
            }
            Err(e) => {
                errors += 1;
                println!("  [ERR ] {e:?}");
            }
        }
    }

    let total = results.len();
    println!();
    println!("场景总数        = {total}");
    println!("定位达标        = {passed}");
    println!("定位未达标      = {failed}");
    println!("执行失败        = {errors}");

    if !capture_times.is_empty() {
        let avg = |v: &[u64]| v.iter().sum::<u64>() as f64 / v.len() as f64 / 1e6;
        println!("捕获平均        = {:.1} ms", avg(&capture_times));
        println!("定位平均        = {:.1} ms", avg(&detect_times));
        println!(
            "最低 IoU        = 标题 {:.3} / 上下文 {:.3} / 输入框 {:.3}",
            min_title, min_context, min_compose
        );
        println!(
            "布局签名种类    = {}（尺寸/主题不同会产生不同签名）",
            signatures.len()
        );
    }

    println!();
    let all_ok = failed == 0 && errors == 0 && passed == total && total > 0;
    println!("门槛 关键 ROI 全部达标 : {}", verdict(all_ok));
    println!("注：这是自有合成窗口上的结果，不代表真实目标程序的布局兼容性。");
    println!("    DPI 档位由当前显示器决定，100/125/150/200% 全矩阵仍需在多显示器环境复跑。");

    if !all_ok {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn e2e_latency(rounds: usize) {
    use m0_windows::harness::e2e_latency::run;

    println!("=== M0-D 合成分析链延迟（自有测试窗口 + 保活 OCR 子进程）===");
    println!("轮数 = {rounds}（前 5 轮为预热，不计入统计）");
    println!();

    let warmup = 5;
    let total = rounds + warmup;
    let durations = match run(total) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("测量失败：{e}");
            std::process::exit(1);
        }
    };

    let measured: Vec<u64> = durations.into_iter().skip(warmup).collect();
    if measured.is_empty() {
        eprintln!("没有有效样本。");
        std::process::exit(1);
    }

    let mut sorted = measured.clone();
    sorted.sort_unstable();
    let pct = |p: f64| -> u64 {
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[idx]
    };
    let mean = measured.iter().sum::<u64>() as f64 / measured.len() as f64 / 1e6;

    println!("样本数        = {}", sorted.len());
    println!("平均          = {mean:.1} ms");
    println!("p50           = {:.1} ms", pct(0.50) as f64 / 1e6);
    println!("p95           = {:.1} ms", pct(0.95) as f64 / 1e6);
    println!("p99           = {:.1} ms", pct(0.99) as f64 / 1e6);
    println!(
        "最大          = {:.1} ms",
        sorted[sorted.len() - 1] as f64 / 1e6
    );
    println!();

    let p95_ok = pct(0.95) <= 800_000_000;
    println!("门槛 分析链 p95 ≤ 800 ms : {}", verdict(p95_ok));
    println!("注：不含物理按键、协调排队与 UI 渲染，不能据此判定 M0-D 通过。");
    println!("    物理按键到提示可见及 100 ms 反馈仍须在 M0 阶段实测。");
    if !p95_ok {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn ocr_export() {
    use m0_windows::harness::ocr_export::export_for_ocr;

    println!("=== M0-C OCR ROI 导出（自有测试窗口，无真实聊天内容）===");
    match export_for_ocr() {
        Ok(result) => {
            println!("导出场景数   = {}", result.cases);
            println!("清单文件     = {}", result.manifest_path.display());
            println!();
            println!("下一步在 python/dc_worker 下运行：");
            println!("  uv run python -m dc_worker ocr-eval ../../artifacts/m0-ocr/manifest.json");
        }
        Err(e) => {
            eprintln!("导出失败：{e}");
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
fn full_latency(rounds: usize, budget_secs: u64) {
    use m0_windows::harness::full_latency::run;

    println!("=== M0-D 完整延迟：物理按键 → 提示可见 ===");
    println!();
    println!("测试窗口即将出现。请**点击该窗口使其成为前台**，然后按 Enter。");
    println!("每按一次 Enter 采集一个样本，共需 {rounds} 个。");
    println!("Enter 会被抑制（这正是产品行为）；消息不会发出，窗口只是合成界面。");
    println!("最多等待 {budget_secs} 秒，超时后按已采集样本统计。");
    println!();

    let timings = match run(rounds, Duration::from_secs(budget_secs)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("测量失败：{e}");
            std::process::exit(1);
        }
    };

    let mut feedback: Vec<u64> = timings.iter().map(|t| t.feedback_nanos).collect();
    let mut total: Vec<u64> = timings.iter().map(|t| t.total_nanos).collect();
    let mut analysis: Vec<u64> = timings.iter().map(|t| t.analysis_nanos).collect();
    feedback.sort_unstable();
    total.sort_unstable();
    analysis.sort_unstable();

    let pct = |v: &[u64], p: f64| -> u64 {
        let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
        v[idx]
    };
    let ms = |n: u64| n as f64 / 1e6;

    println!();
    println!("样本数                = {}", timings.len());
    println!();
    println!("--- 按键 → 「正在检查」可见 ---");
    println!("p50                   = {:.1} ms", ms(pct(&feedback, 0.50)));
    println!("p95                   = {:.1} ms", ms(pct(&feedback, 0.95)));
    println!(
        "最大                  = {:.1} ms",
        ms(feedback[feedback.len() - 1])
    );
    println!();
    println!("--- 按键 → 结果提示可见 ---");
    println!("p50                   = {:.1} ms", ms(pct(&total, 0.50)));
    println!("p95                   = {:.1} ms", ms(pct(&total, 0.95)));
    println!("p99                   = {:.1} ms", ms(pct(&total, 0.99)));
    println!(
        "最大                  = {:.1} ms",
        ms(total[total.len() - 1])
    );
    println!();
    println!("其中分析耗时 p50      = {:.1} ms", ms(pct(&analysis, 0.50)));
    println!();

    // 100 ms 反馈是对每次事务的承诺，取最大值判定而非分位数。
    let feedback_ok = feedback[feedback.len() - 1] <= 100_000_000;
    let total_ok = pct(&total, 0.95) <= 800_000_000;
    let enough = timings.len() >= 20;

    println!("门槛 反馈 ≤ 100 ms（全部样本）: {}", verdict(feedback_ok));
    println!("门槛 提示可见 p95 ≤ 800 ms    : {}", verdict(total_ok));
    println!("样本量 ≥ 20                   : {}", verdict(enough));
    println!();
    println!("注：目标为自有合成窗口，不代表真实微信的捕获与定位兼容性。");

    if !(feedback_ok && total_ok && enough) {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn ime_probe() {
    use m0_windows::platform::ime::{probe_foreground, ImeState};

    println!("=== M0-B 输入法组合状态探测 ===");
    println!("每秒采样一次前台窗口的输入法状态，共 30 秒。");
    println!("请在任意可输入的窗口中轮流执行：");
    println!("  1. 纯英文直接输入");
    println!("  2. 微软拼音/五笔输入拼音但不确认候选");
    println!("  3. 候选框打开时停住");
    println!("  4. 确认候选后继续英文");
    println!();

    for i in 1..=30 {
        let state = probe_foreground();
        let label = match state {
            ImeState::NotComposing => "未组合（可判定发送键）",
            ImeState::Composing => "组合中（Enter 属于输入法，必须透传）",
            ImeState::Unknown => "不确定（按透传处理）",
        };
        println!("  [{i:>2}/30] {state:?}  {label}");
        std::thread::sleep(Duration::from_secs(1));
    }

    println!();
    println!("说明：Unknown 是安全默认值——无法证明没有组合时宁可透传。");
    println!("      若长时间停在 Unknown 而无法进入 Composing，说明 IMM32 路径");
    println!("      对当前输入法（可能使用 TSF）不可见，M0-B 需改用 TSF 探测或");
    println!("      收窄首期支持矩阵为 Ctrl+Enter。");
}

#[cfg(windows)]
fn ime_hook_probe(secs: u64) {
    use m0_windows::domain::keys::Shortcut;
    use m0_windows::platform::hook::{run_hook_thread, GuardCommand, HookShared};
    use m0_windows::platform::ime::{probe_foreground, ImeState};

    let shared = HookShared::new();
    let worker = {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("dc-hook".into())
            .spawn(move || run_hook_thread(shared, Shortcut::CtrlEnter, 1))
            .expect("无法创建钩子线程")
    };

    let deadline = Instant::now() + Duration::from_secs(5);
    while !shared.installed.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !shared.installed.load(Ordering::Acquire) {
        eprintln!("钩子安装失败。");
        std::process::exit(1);
    }

    println!("=== M0-B IME → Hook 状态发布观察（{secs} 秒）===");
    println!("协调线程每 100 ms 查询前台 IME；Hook 回调只读取已发布的布尔状态。");
    println!("本命令不 arm 目标，所有按键都应透传。请切换输入法并观察状态迁移。");

    let until = Instant::now() + Duration::from_secs(secs);
    let mut previous: Option<ImeState> = None;
    while Instant::now() < until {
        let state = probe_foreground();
        if previous != Some(state) {
            let composing = state.hook_composing();
            shared.send(GuardCommand::SetImeComposing(composing));
            println!("  {state:?} → hook ime_composing={composing}");
            previous = Some(state);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    shared.request_stop();
    let _ = worker.join();
    let suppressed = shared.metrics.events_suppressed.load(Ordering::Relaxed);
    println!("抑制事件 = {suppressed}（未 arm 目标时必须为 0）");
    if suppressed != 0 {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn hook_latency(secs: u64) {
    use m0_windows::domain::keys::Shortcut;
    use m0_windows::platform::hook::{run_hook_thread, HookShared};

    let shared = HookShared::new();
    let worker = {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("dc-hook".into())
            .spawn(move || run_hook_thread(shared, Shortcut::CtrlEnter, 1))
            .expect("无法创建钩子线程")
    };

    // 等待钩子安装完成。
    let deadline = Instant::now() + Duration::from_secs(5);
    while !shared.installed.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !shared.installed.load(Ordering::Acquire) {
        eprintln!("钩子安装失败。");
        std::process::exit(1);
    }

    println!("低级键盘钩子已安装 {secs} 秒。");
    println!("本模式未 arm 任何目标，因此所有按键都应透传；请正常打字以产生样本。");
    println!("（正在计时……）");

    std::thread::sleep(Duration::from_secs(secs));
    shared.request_stop();
    let _ = worker.join();

    let m = &shared.metrics;
    let seen = m.events_seen.load(Ordering::Relaxed);
    let passed = m.events_passed.load(Ordering::Relaxed);
    let suppressed = m.events_suppressed.load(Ordering::Relaxed);
    let samples = shared.latency_samples();

    println!();
    println!("=== M0-A 回调延迟与透传统计 ===");
    println!("事件总数          = {seen}");
    println!("透传              = {passed}");
    println!("抑制              = {suppressed}  (未 arm 目标时必须为 0)");

    if samples.is_empty() {
        eprintln!("没有采集到延迟样本，M0-A 门槛未验证。");
        std::process::exit(1);
    }

    let pct = |p: f64| -> u32 {
        let idx = ((samples.len() as f64 - 1.0) * p).round() as usize;
        samples[idx]
    };
    let total: u64 = samples.iter().map(|v| *v as u64).sum();
    println!("样本数            = {}", samples.len());
    println!(
        "平均              = {:.1} µs",
        total as f64 / samples.len() as f64 / 1000.0
    );
    println!("p50               = {:.1} µs", pct(0.50) as f64 / 1000.0);
    println!("p95               = {:.1} µs", pct(0.95) as f64 / 1000.0);
    println!("p99               = {:.1} µs", pct(0.99) as f64 / 1000.0);
    println!(
        "最大              = {:.1} µs",
        samples[samples.len() - 1] as f64 / 1000.0
    );
    println!();

    let p99_ns = pct(0.99) as u64;
    let max_ns = samples[samples.len() - 1] as u64;
    let p99_ok = p99_ns <= 250_000;
    let max_ok = max_ns <= 2_000_000;
    let no_false_suppression = suppressed == 0;

    println!("门槛 p99 ≤ 0.25 ms       : {}", verdict(p99_ok));
    println!("门槛 单次最大 ≤ 2 ms     : {}", verdict(max_ok));
    println!(
        "门槛 非目标零误阻止      : {}",
        verdict(no_false_suppression)
    );

    if !(p99_ok && max_ok && no_false_suppression) {
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn verdict(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}
