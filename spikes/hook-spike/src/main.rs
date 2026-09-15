// 复刻 dc-sys 的 sys-thread 模式：SetWinEventHook + WH_KEYBOARD_LL + PeekMessage 泵
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HWND, LRESULT, WPARAM, LPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, 
    DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, SetWindowsHookExW,
    TranslateMessage, KBDLLHOOKSTRUCT, MSG, PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_SYSKEYDOWN, WM_KEYUP, WM_SYSKEYUP, MWMO_INPUTAVAILABLE,
};

static FG_COUNT: AtomicU64 = AtomicU64::new(0);
static KEY_COUNT: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn foreground_proc(
    _h: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _a: i32,
    _b: i32,
    _t: u32,
    _time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        FG_COUNT.fetch_add(1, Ordering::SeqCst);
        println!("[FG] 前台事件 hwnd={:?}", hwnd.0);
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN || msg == WM_KEYUP || msg == WM_SYSKEYUP {
            let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            KEY_COUNT.fetch_add(1, Ordering::SeqCst);
            println!("[KEY] vk=0x{:02X} down={}", kb.vkCode, msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN);
        }
    }
    unsafe { windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam) }
}

fn main() {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);

    let t = std::thread::Builder::new()
        .name("sys-thread".into())
        .spawn(move || {
            unsafe {
                // 与 dc-sys 完全一致的安装方式
                let key_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0)
                    .expect("键盘钩子安装失败");
                println!("键盘钩子已安装");
                let fg_hook = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
                if fg_hook.is_invalid() { panic!("前台钩子安装失败"); }
                println!("前台钩子已安装");

                let event = windows::Win32::System::Threading::CreateEventW(None, false, false, windows::core::PCWSTR::null()).unwrap();
                let mut msg = MSG::default();
                while !stop2.load(Ordering::SeqCst) {
                    unsafe {
                        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                        MsgWaitForMultipleObjectsEx(
                            Some(&[HANDLE(event.0 as *mut core::ffi::c_void)]),
                            200,
                            QS_ALLINPUT,
                            MWMO_INPUTAVAILABLE,
                        );
                    }
                }
                let _ = UnhookWinEvent(fg_hook);
                let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(key_hook);
            }
        })
        .unwrap();

    println!("运行 20 秒——期间请切换窗口、敲键盘……");
    std::thread::sleep(Duration::from_secs(20));
    stop.store(true, Ordering::SeqCst);
    let _ = t.join();
    println!(
        "结果：前台事件 {} 次，按键事件 {} 次",
        FG_COUNT.load(Ordering::SeqCst),
        KEY_COUNT.load(Ordering::SeqCst)
    );
}
