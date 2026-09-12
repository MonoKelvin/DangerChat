//! 显示器级屏幕捕获与目标窗口裁剪。
//!
//! 合规边界（`docs/合规与风险说明.md` §4.1）：
//! - 只允许显示器级捕获接口，先取整屏再按 DWM 窗口边界裁剪。
//! - 禁止窗口定向捕获、`PrintWindow`、UIA，以及任何要求目标窗口自行渲染的调用。
//! - 只在一次有效发送事务中取帧；不做固定频率轮询，不持续录屏。
//! - 原始帧只在内存中存活一次分析的时间，默认不落盘。
//!
//! 本模块目前实现降级链的最后一级 GDI，用于在 M0 阶段验证坐标、DPI 与裁剪正确性。
//! DXGI Desktop Duplication 与 WGC 显示器捕获在通过本级验证后接入，
//! 三者共享同一个 `MonitorFrame` 输出契约。

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    SRCCOPY,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// 捕获失败原因。任何一种都不得被当作“内容安全”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    /// 没有可用的目标显示器。
    NoMonitor,
    /// 目标窗口与显示器无交集，或窗口最小化。
    NotOnScreen,
    /// 裁剪区域为空或超出显示器范围。
    EmptyRegion,
    /// GDI/DXGI 调用失败。
    DeviceFailure,
    /// 取到全黑帧：可能被保护内容或驱动拒绝。
    BlackFrame,
}

/// 单次捕获得到的目标区域像素。BGRA8，行优先，自上而下。
pub struct TargetFrame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 相对桌面坐标的裁剪原点，用于把 ROI 换算回屏幕位置。
    pub origin: POINT,
    pub monitor_bounds: RECT,
    pub pixels: Vec<u8>,
    pub capture_nanos: u64,
}

impl TargetFrame {
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    /// 是否整帧近似全黑。用于识别被拒绝的捕获，不做内容判断。
    pub fn is_black(&self) -> bool {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| px[0] < 8 && px[1] < 8 && px[2] < 8)
    }
}

/// 声明 Per-Monitor V2 DPI 感知。必须在任何窗口或捕获调用之前执行一次。
pub fn declare_dpi_awareness() {
    // SAFETY: 进程级一次性设置，失败时（已由清单声明）忽略即可。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 取得窗口所在显示器的桌面坐标范围。
pub fn monitor_bounds_for_window(hwnd: u64) -> Option<RECT> {
    // SAFETY: 只读显示器信息；传入的是系统返回的窗口句柄。
    unsafe {
        let hmon = MonitorFromWindow(
            windows::Win32::Foundation::HWND(hwnd as *mut _),
            MONITOR_DEFAULTTONEAREST,
        );
        monitor_info(hmon)
    }
}

fn monitor_info(hmon: HMONITOR) -> Option<RECT> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: 传入本地结构体指针并已设置 cbSize。
    let ok = unsafe { GetMonitorInfoW(hmon, &mut info) };
    if ok.as_bool() {
        Some(info.rcMonitor)
    } else {
        None
    }
}

/// 捕获窗口所在显示器，并按给定矩形裁剪出目标区域。
///
/// `region` 应为窗口客户区的桌面坐标（`GetClientRect` + `ClientToScreen`）。
/// 使用客户区而非 DWM 扩展边界，是为了把窗口自身的标题栏与边框排除在分析范围之外；
/// 两者都属于操作系统窗口管理接口，不读取目标程序的任何内容。
pub fn capture_window_region(
    hwnd: u64,
    region: RECT,
    clock: &crate::platform::clock::MonotonicClock,
) -> Result<TargetFrame, CaptureError> {
    let started = clock.now_nanos();
    let monitor = monitor_bounds_for_window(hwnd).ok_or(CaptureError::NoMonitor)?;

    // 目标区域与显示器求交，得到本显示器上实际可见的部分。
    let clip = RECT {
        left: region.left.max(monitor.left),
        top: region.top.max(monitor.top),
        right: region.right.min(monitor.right),
        bottom: region.bottom.min(monitor.bottom),
    };
    let width = clip.right - clip.left;
    let height = clip.bottom - clip.top;
    if width <= 0 || height <= 0 {
        return Err(CaptureError::NotOnScreen);
    }
    if width > 4096 || height > 4096 {
        // 与 Worker ROI 上限保持一致，超限由调用方分块处理。
        return Err(CaptureError::EmptyRegion);
    }

    let pixels = blit_region(clip, width, height)?;
    let frame = TargetFrame {
        width: width as u32,
        height: height as u32,
        stride: (width * 4) as u32,
        origin: POINT {
            x: clip.left,
            y: clip.top,
        },
        monitor_bounds: monitor,
        pixels,
        capture_nanos: clock.now_nanos().saturating_sub(started),
    };
    if frame.is_black() {
        return Err(CaptureError::BlackFrame);
    }
    Ok(frame)
}

/// GDI 降级路径：从屏幕 DC 复制指定矩形。只读取已渲染的桌面像素。
fn blit_region(clip: RECT, width: i32, height: i32) -> Result<Vec<u8>, CaptureError> {
    // SAFETY: 以下 GDI 对象在函数退出前全部释放；所有指针均指向本地缓冲区。
    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(CaptureError::DeviceFailure);
        }
        let guard = ScreenDc(screen_dc);

        let mem_dc = CreateCompatibleDC(Some(guard.0));
        if mem_dc.is_invalid() {
            return Err(CaptureError::DeviceFailure);
        }
        let bitmap = CreateCompatibleBitmap(guard.0, width, height);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            return Err(CaptureError::DeviceFailure);
        }
        let previous = SelectObject(mem_dc, bitmap.into());

        let blit_ok = BitBlt(
            mem_dc,
            0,
            0,
            width,
            height,
            Some(guard.0),
            clip.left,
            clip.top,
            SRCCOPY,
        )
        .is_ok();

        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // 负高度表示自上而下的行序，避免后续再翻转。
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        let copied = if blit_ok {
            GetDIBits(
                mem_dc,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut header,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(mem_dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);

        if copied == 0 {
            return Err(CaptureError::DeviceFailure);
        }
        Ok(pixels)
    }
}

/// 屏幕 DC 的释放守卫。
struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: 与 GetDC(None) 配对释放。
        unsafe {
            ReleaseDC(None, self.0);
        }
    }
}

/// 主显示器范围，供无窗口场景使用。
pub fn primary_monitor_bounds() -> Option<RECT> {
    // SAFETY: 只读显示器信息。
    unsafe {
        monitor_info(MonitorFromPoint(
            POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTONEAREST,
        ))
    }
}
