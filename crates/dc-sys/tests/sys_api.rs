//! UT-SYS-01 ~ UT-SYS-02：平台层契约（设计文档 §5.2 测试矩阵）
//!
//! 全部用 `MockSys`，不需要真实桌面（§11.1）。真机链路见 `IT-SYS-01`（手动、`--ignored`）。

use dc_sys::{
    dpi_scale_from_dpi, to_physical, ForegroundInfo, HookAction, Hwnd, KeyEvent, MockSys,
    MockWindow, Rect, SysApi, VK_PROCESSKEY,
};
use image::RgbaImage;

const DIP: Rect = Rect {
    x: 10,
    y: 20,
    w: 300,
    h: 200,
};

fn hwnd(n: isize) -> Hwnd {
    Hwnd(n)
}

fn enter_event() -> KeyEvent {
    KeyEvent {
        vk: 0x0D,
        scan_code: 0x1C,
        is_key_down: true,
        is_injected: false,
        ctrl: false,
        alt: false,
        shift: false,
    }
}

/// UT-SYS-01 MockSys 注入窗口序列，坐标/DPI 换算正确（1.0/1.25/1.5）
#[test]
fn ut_sys_01_window_rect_dpi_conversion() {
    let mock = MockSys::new();
    mock.set_window(hwnd(1), MockWindow::new(DIP, 1.0));
    mock.set_window(hwnd(2), MockWindow::new(DIP, 1.25));
    mock.set_window(hwnd(3), MockWindow::new(DIP, 1.5));

    let (r1, s1) = mock.window_rect(hwnd(1)).unwrap();
    let (r2, s2) = mock.window_rect(hwnd(2)).unwrap();
    let (r3, s3) = mock.window_rect(hwnd(3)).unwrap();

    assert_eq!((s1, s2, s3), (1.0, 1.25, 1.5));
    assert_eq!(r1, Rect::new(10, 20, 300, 200), "100% 缩放原样输出");
    assert_eq!(r2, Rect::new(13, 25, 375, 250), "125%：左上与右下分别取整");
    assert_eq!(r3, Rect::new(15, 30, 450, 300), "150%");

    // 右下角不变式：x+w 由右下角取整得出（避免逐次缩放丢 1px）
    for scale in [1.0f32, 1.25, 1.5] {
        let rect = to_physical(DIP, scale);
        assert_eq!(rect.x, (DIP.x as f32 * scale).round() as i32);
        assert_eq!(rect.y, (DIP.y as f32 * scale).round() as i32);
        assert_eq!(
            rect.right(),
            ((DIP.x + DIP.w as i32) as f32 * scale).round() as i32,
            "右边界按同一规则取整（scale={scale}）"
        );
        assert_eq!(
            rect.bottom(),
            ((DIP.y + DIP.h as i32) as f32 * scale).round() as i32
        );
    }
    assert_eq!(to_physical(DIP, 1.0), DIP, "100% 缩放原样输出");
    assert_eq!(
        to_physical(Rect::new(0, 0, 0, 0), 1.5),
        Rect::new(0, 0, 0, 0)
    );

    // DPI → scale
    assert_eq!(dpi_scale_from_dpi(96), 1.0);
    assert_eq!(dpi_scale_from_dpi(120), 1.25);
    assert_eq!(dpi_scale_from_dpi(144), 1.5);
    assert_eq!(dpi_scale_from_dpi(0), 1.0, "取不到 DPI 时按 100% 保守处理");

    // 未登记的窗口 → 明确错误，而非假矩形
    assert!(mock.window_rect(hwnd(99)).is_err());

    // 截图使用物理矩形（不是 DIP 矩形）：像素尺寸与坐标都要对得上
    let mut screen = RgbaImage::new(2000, 1200);
    for y in 0..1200 {
        for x in 0..2000 {
            screen.put_pixel(
                x,
                y,
                image::Rgba([(x % 256) as u8, (y % 256) as u8, 7, 255]),
            );
        }
    }
    mock.set_screen(screen.clone());
    let shot = mock.capture_region(r2).expect("capture at 125%");
    assert_eq!(shot.dimensions(), (375, 250), "物理像素尺寸");
    assert_eq!(
        *shot.get_pixel(0, 0),
        *screen.get_pixel(13, 25),
        "裁剪原点对齐物理坐标"
    );
    assert_eq!(*shot.get_pixel(374, 249), *screen.get_pixel(387, 274));

    // 最小化窗口
    mock.set_minimized(hwnd(1), true);
    assert!(mock.is_window_minimized(hwnd(1)));
    assert!(mock.is_window_minimized(hwnd(1234)), "未登记窗口视为不可用");
}

/// UT-SYS-02 IME 组合态 mock：组合中发送键必须放行，且系统信号零成本可判
/// （「放行发送键 + 草稿纪元 +1」的完整断言属 UT-INT-07，M2 在 intercept 层联动验证）
#[test]
fn ut_sys_02_ime_composing_signal() {
    let mock = MockSys::new();
    assert!(!mock.ime_composing(), "默认非组合态");

    mock.set_ime_composing(true);
    assert!(mock.ime_composing());

    // VK_PROCESSKEY 是被输入法消费的键的系统信号（0xE5）
    let consumed = KeyEvent {
        vk: VK_PROCESSKEY,
        scan_code: 0x1C,
        is_key_down: true,
        is_injected: false,
        ctrl: false,
        alt: false,
        shift: false,
    };
    assert!(consumed.is_ime_consumed());
    assert!(!enter_event().is_ime_consumed(), "回车不是输入法消费键");

    // 组合态下钩子回调仍然被投递（由 intercept 决定放行），系统层不吞键
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&calls);
    let _guard = mock
        .install_keyboard_hook(Box::new(move |_ev| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            HookAction::Pass
        }))
        .expect("install hook");

    assert_eq!(mock.feed_key(consumed), HookAction::Pass);
    assert_eq!(
        mock.feed_key(enter_event()),
        HookAction::Pass,
        "组合态下回车放行"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);

    mock.set_ime_composing(false);
    assert!(!mock.ime_composing());
}

/// 钩子/监听守卫的生命周期：Drop 即卸载（防止测试或热重载后残留钩子）
#[test]
fn hook_and_watch_guards_teardown_on_drop() {
    let mock = MockSys::new();
    {
        let _hook = mock
            .install_keyboard_hook(Box::new(|_| HookAction::Swallow))
            .unwrap();
        assert!(mock.is_hooked());
        let _watch = mock.watch_foreground(Box::new(|_| {})).unwrap();
        assert!(mock.watch_installed());
    }
    assert!(!mock.is_hooked(), "HookGuard 释放后钩子已卸载");
    assert!(!mock.watch_installed(), "WatchGuard 释放后监听已取消");

    // 未安装钩子时投递按键 → 放行（绝不吞键）
    assert_eq!(mock.feed_key(enter_event()), HookAction::Pass);
}

/// 多回调语义：按注册顺序分发、任一 Swallow 即终止；各自卸载互不影响
///
/// 回归用例：曾用单槽 thread-local 存回调，导致装第二个钩子时**静默顶替**第一个回调，
/// 观察者永远收不到事件（实测表现为「放行按键 = 0、按键总数翻倍」）。
#[test]
fn multiple_keyboard_callbacks_chain_and_short_circuit() {
    let mock = MockSys::new();
    let seen_a = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_b = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let a = std::sync::Arc::clone(&seen_a);
    let guard_a = mock
        .install_keyboard_hook(Box::new(move |ev: &KeyEvent| {
            a.lock().unwrap().push(ev.vk);
            if ev.vk == 0x0D {
                HookAction::Swallow
            } else {
                HookAction::Pass
            }
        }))
        .expect("注册 A");
    let b = std::sync::Arc::clone(&seen_b);
    let guard_b = mock
        .install_keyboard_hook(Box::new(move |ev: &KeyEvent| {
            b.lock().unwrap().push(ev.vk);
            HookAction::Pass
        }))
        .expect("注册 B");

    assert_eq!(mock.key_callback_count(), 2, "两者都应被登记，不得互相顶替");

    let letter = |vk: u16| KeyEvent {
        vk,
        scan_code: 0,
        is_key_down: true,
        is_injected: false,
        ctrl: false,
        alt: false,
        shift: false,
    };

    // 放行键：两个回调都收到
    assert_eq!(mock.feed_key(letter(0x41)), HookAction::Pass);
    assert_eq!(*seen_a.lock().unwrap(), vec![0x41]);
    assert_eq!(*seen_b.lock().unwrap(), vec![0x41]);

    // 吞掉的键：分发在 A 处终止，B 收不到
    assert_eq!(mock.feed_key(enter_event()), HookAction::Swallow);
    assert_eq!(*seen_a.lock().unwrap(), vec![0x41, 0x0D]);
    assert_eq!(*seen_b.lock().unwrap(), vec![0x41], "吞键后不得继续分发");

    // A 卸载后钩子仍在（B 还在），且回车改为放行
    drop(guard_a);
    assert_eq!(mock.key_callback_count(), 1);
    assert!(mock.is_hooked());
    assert_eq!(mock.feed_key(enter_event()), HookAction::Pass);
    assert_eq!(*seen_b.lock().unwrap(), vec![0x41, 0x0D]);

    // 最后一个回调卸载后钩子才真正卸载
    drop(guard_b);
    assert_eq!(mock.key_callback_count(), 0);
    assert!(!mock.is_hooked());
}

/// 前台监听同样支持多消费者，且各自卸载互不影响
#[test]
fn multiple_foreground_watchers() {
    let mock = MockSys::new();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let first = std::sync::Arc::clone(&hits);
    let g1 = mock
        .watch_foreground(Box::new(move |_| {
            first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }))
        .unwrap();
    let second = std::sync::Arc::clone(&hits);
    let g2 = mock
        .watch_foreground(Box::new(move |_| {
            second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }))
        .unwrap();
    assert_eq!(mock.foreground_callback_count(), 2);

    mock.emit_foreground(ForegroundInfo {
        hwnd: hwnd(1),
        pid: 2,
        process_name: "notepad.exe".to_string(),
    });
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "两个监听都该收到"
    );

    drop(g1);
    assert_eq!(mock.foreground_callback_count(), 1);
    mock.emit_foreground(ForegroundInfo {
        hwnd: hwnd(1),
        pid: 2,
        process_name: "notepad.exe".to_string(),
    });
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);
    drop(g2);
    assert!(!mock.watch_installed());
}

/// 前台事件投递：回调拿到完整信息，且不影响 `foreground()` 查询
#[test]
fn foreground_watch_dispatches_events() {
    let mock = MockSys::new();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);
    let _watch = mock
        .watch_foreground(Box::new(move |info| {
            sink.lock().unwrap().push(info.process_name);
        }))
        .unwrap();

    let info = ForegroundInfo {
        hwnd: hwnd(42),
        pid: 1000,
        process_name: "notepad.exe".to_string(),
    };
    mock.emit_foreground(info.clone());
    assert_eq!(mock.foreground(), Some(info));
    assert_eq!(mock.foreground_event_count(), 1);
    assert_eq!(seen.lock().unwrap().as_slice(), ["notepad.exe"]);
}

/// 截图失败路径：矩形越界/未配置屏幕/强制失败都必须返回错误，绝不返回假图
#[test]
fn capture_failures_are_explicit() {
    let mock = MockSys::new();
    assert!(
        mock.capture_region(Rect::new(0, 0, 10, 10)).is_err(),
        "未配置屏幕"
    );

    mock.set_screen(RgbaImage::new(100, 100));
    assert!(
        mock.capture_region(Rect::new(500, 500, 10, 10)).is_err(),
        "完全越界"
    );
    assert!(
        mock.capture_region(Rect::new(0, 0, 0, 0)).is_err(),
        "空矩形"
    );

    // 部分越界 → 裁剪到屏幕内（前台窗口不会被整体遮挡，局部遮挡属可接受边角）
    let shot = mock.capture_region(Rect::new(95, 95, 20, 20)).unwrap();
    assert_eq!(shot.dimensions(), (5, 5));

    mock.fail_capture(true);
    assert!(
        mock.capture_region(Rect::new(0, 0, 10, 10)).is_err(),
        "强制失败"
    );
    mock.fail_capture(false);
    mock.fail_capture_after(0);
    assert!(
        mock.capture_region(Rect::new(0, 0, 10, 10)).is_err(),
        "第 1 次起失败"
    );
}
