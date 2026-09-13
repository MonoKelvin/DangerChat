//! 前台窗口观察：只读取操作系统已持有的窗口元数据。
//!
//! 绝不枚举进程、不读取目标 EXE 版本资源、不访问其内存或模块。

use windows::Win32::Foundation::{CloseHandle, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClientRect, GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, GA_ROOTOWNER,
};

use crate::domain::guard::TargetInstance;

/// 前台窗口快照。所有字段都来自操作系统窗口管理接口。
#[derive(Debug, Clone, Copy)]
pub struct ForegroundSnapshot {
    pub instance: TargetInstance,
    /// DWM 扩展边界（桌面坐标）。含窗口自身装饰。
    pub bounds: RECT,
    /// 客户区（桌面坐标）。不含标题栏与边框，是区域定位的分析范围。
    pub client: RECT,
    pub dpi: u32,
    pub minimized: bool,
    pub visible: bool,
}

impl ForegroundSnapshot {
    pub fn width(&self) -> i32 {
        self.bounds.right - self.bounds.left
    }

    pub fn height(&self) -> i32 {
        self.bounds.bottom - self.bounds.top
    }
}

/// 读取当前前台窗口快照。没有前台窗口时返回 `None`。
pub fn foreground_snapshot() -> Option<ForegroundSnapshot> {
    // SAFETY: GetForegroundWindow 无参数、只读。
    let hwnd = unsafe { GetForegroundWindow() };
    snapshot_window(hwnd)
}

/// 读取指定窗口的快照。
///
/// 该函数只查询操作系统窗口元数据。测试设施用它绑定自己创建的窗口，避免终端或
/// 测试运行器抢占前台焦点后误捕获其他窗口。
pub fn snapshot_window(hwnd: HWND) -> Option<ForegroundSnapshot> {
    if hwnd.is_invalid() {
        return None;
    }

    // SAFETY: 以下全部是只读窗口管理调用，参数为调用方提供的 HWND 与本地栈变量。
    unsafe {
        let root = GetAncestor(hwnd, GA_ROOTOWNER);
        let root = if root.is_invalid() { hwnd } else { root };

        let mut pid = 0u32;
        GetWindowThreadProcessId(root, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let bounds = extended_frame_bounds(root)?;
        let client = client_rect_on_screen(root)?;
        let dpi = GetDpiForWindow(root);

        Some(ForegroundSnapshot {
            instance: TargetInstance {
                root_hwnd: root.0 as u64,
                pid,
                process_start_time: process_start_time(pid).unwrap_or(0),
            },
            bounds,
            client,
            dpi: if dpi == 0 { 96 } else { dpi },
            minimized: IsIconic(root).as_bool(),
            visible: IsWindowVisible(root).as_bool(),
        })
    }
}

/// 读取窗口标题。仅用于诊断输出，不参与任何判定逻辑。
pub fn window_title(hwnd: u64) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: GetWindowTextW 写入本地缓冲区并受容量限制，只读窗口属性。
    let len = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(
            windows::Win32::Foundation::HWND(hwnd as *mut _),
            &mut buffer,
        )
    };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

/// 客户区在桌面坐标中的矩形。
///
/// `GetClientRect` 与 `ClientToScreen` 同属操作系统窗口管理接口，
/// 只返回窗口几何信息，不读取目标程序的任何内容、控件树或数据。
/// 区域定位以客户区为分析范围，避免把窗口自身的标题栏与边框误判为界面结构。
fn client_rect_on_screen(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY: 两个调用都只写入本地栈变量，作用于系统返回的窗口句柄。
    unsafe {
        GetClientRect(hwnd, &mut rect).ok()?;
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return None;
        }
    }
    Some(RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + (rect.right - rect.left),
        bottom: origin.y + (rect.bottom - rect.top),
    })
}

/// DWM 扩展边界。比 `GetWindowRect` 更贴合窗口真实可见区域。
fn extended_frame_bounds(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: 传入本地 RECT 的指针与其精确大小，DwmGetWindowAttribute 只写该缓冲区。
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    hr.ok()?;
    Some(rect)
}
/// 进程启动时间。用于区分 HWND/PID 复用后的不同进程实例。
///
/// 只使用 `PROCESS_QUERY_LIMITED_INFORMATION`，不读取内存、模块或文件。
fn process_start_time(pid: u32) -> Option<u64> {
    // SAFETY: OpenProcess 返回的句柄在本函数末尾显式关闭；
    // GetProcessTimes 只写入我们提供的四个栈变量。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut creation = Default::default();
        let mut exit = Default::default();
        let mut kernel = Default::default();
        let mut user = Default::default();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        Some(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
    }
}
