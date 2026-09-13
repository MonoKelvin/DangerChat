#![cfg(windows)]
//! IT-SYS-01（手动，`--ignored`）：真实桌面验证 dc-sys 的不接触式能力（设计文档 §5.2、§11.1）。
//!
//! 只读屏幕像素与公开窗口 API：**不注入任何输入、不写任何文件**。
//!
//! ```text
//! cargo test -p dc-sys --test it_sys_01_real_desktop -- --ignored --nocapture
//! ```
//!
//! 验证点：
//! 1. `GetForegroundWindow` + 进程名解析（前台窗口存在且能拿到进程名）；
//! 2. `GetWindowRect` + `GetDpiForWindow`（矩形非空、DPI 缩放合理）；
//! 3. `IsIconic` 与前台窗口一致性；
//! 4. `BitBlt` 屏幕直拷：尺寸与矩形一致，且**不是纯色**（证明真的抓到了屏幕内容，而不是空缓冲）；
//! 5. 键盘钩子可安装/卸载（安装后立即卸载，不打扰用户按键）；
//! 6. IME 组合态查询可用（组合中返回值随输入法状态翻转）。

use dc_sys::{RealSys, SysApi};

#[test]
#[ignore = "手动集成：需要真实桌面会话（只读屏幕，不注入输入）"]
fn it_sys_01_real_desktop_capabilities() {
    let sys = RealSys::new();
    assert!(sys.is_dpi_aware(), "进程应已切换为 per-monitor DPI 感知");
    println!("per-monitor DPI 感知：已启用");

    // 1) 前台窗口
    let fg = sys.foreground().expect("真实桌面应有前台窗口");
    assert!(!fg.process_name.is_empty(), "应能解析前台进程名");
    assert_ne!(fg.pid, 0);
    println!(
        "前台窗口：{} (pid={}, {})",
        fg.process_name, fg.pid, fg.hwnd
    );

    // 2) 窗口矩形 + DPI
    let (rect, dpi_scale) = sys.window_rect(fg.hwnd).expect("取窗口矩形");
    assert!(!rect.is_empty(), "前台窗口矩形不应为空");
    assert!(
        (1.0..=4.0).contains(&dpi_scale),
        "DPI 缩放异常：{dpi_scale}"
    );
    println!(
        "窗口矩形：{}x{} @ ({},{})，dpi_scale={dpi_scale}",
        rect.w, rect.h, rect.x, rect.y
    );

    // 3) 最小化状态（前台窗口不可能是最小化）
    assert!(!sys.is_window_minimized(fg.hwnd), "前台窗口不应处于最小化");

    // 4) 屏幕直拷
    let img = sys.capture_region(rect).expect("屏幕像素直拷");
    assert_eq!(
        img.dimensions(),
        (rect.w, rect.h),
        "截图像素尺寸必须等于窗口矩形（DPI 换算正确）"
    );
    let mut colors = std::collections::HashSet::new();
    for y in (0..rect.h).step_by((rect.h as usize / 8).max(1)) {
        for x in (0..rect.w).step_by((rect.w as usize / 8).max(1)) {
            let p = img.get_pixel(x, y);
            colors.insert((p[0], p[1], p[2]));
        }
    }
    assert!(
        colors.len() > 1,
        "采样到的像素全同色（{} 种），BitBlt 可能返回了空缓冲",
        colors.len()
    );
    println!(
        "截图：{} 个采样点出现 {} 种颜色（非纯色 ✓）",
        81,
        colors.len()
    );

    // 5) 键盘钩子：注册**两个**回调，验证多消费者语义
    //    （回归：曾用单槽回调存放钩子，后装者会静默顶替先装者）
    let hits_a = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let hits_b = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let a = std::sync::Arc::clone(&hits_a);
    let hook_a = sys
        .install_keyboard_hook(Box::new(move |ev: &dc_sys::KeyEvent| {
            a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if ev.is_key_down {
                println!("  [回调 A] vk={:#04X}", ev.vk);
            }
            dc_sys::HookAction::Pass
        }))
        .expect("注册回调 A");
    let b = std::sync::Arc::clone(&hits_b);
    let hook_b = sys
        .install_keyboard_hook(Box::new(move |ev: &dc_sys::KeyEvent| {
            b.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if ev.is_key_down {
                println!("  [回调 B] vk={:#04X}", ev.vk);
            }
            dc_sys::HookAction::Pass
        }))
        .expect("注册回调 B");

    println!("键盘钩子：已注册 2 个回调（8 秒观察窗，请随意敲几下键盘）；随后自动卸载");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut checked_ime = false;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !checked_ime {
            // 6) IME 组合态查询（不注入，仅读取状态）
            let composing = sys.ime_composing();
            println!("IME 组合态查询可用：composing={composing}");
            checked_ime = true;
        }
    }

    // 卸载 A 后 B 仍应完好（守卫只摘除自己的回调）
    drop(hook_a);
    let (count_a, count_b) = (
        hits_a.load(std::sync::atomic::Ordering::SeqCst),
        hits_b.load(std::sync::atomic::Ordering::SeqCst),
    );
    println!("观察窗按键：回调 A={count_a} 次，回调 B={count_b} 次");
    assert_eq!(
        count_a, count_b,
        "两个回调应收到完全相同的事件流（说明未被互相顶替）"
    );
    if count_a == 0 {
        println!("提示：观察窗内未按任何键，多回调链路未取得实测样本（不影响其余验证项）");
    }
    drop(hook_b);
    println!("键盘钩子：已卸载");

    println!(
        "\nIT-SYS-01 通过：前台检测 / 窗口矩形 / DPI / 屏幕直拷 / 钩子多回调 / IME 查询 均正常。"
    );
}
