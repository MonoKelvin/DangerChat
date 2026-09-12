//! 自有测试聊天窗口。
//!
//! 用途（`docs/03_危信v1.0_工程测试与上线手册.md` §7.5）：
//! 提供一个结构与真实聊天软件相似、但内容完全合成的窗口，
//! 用来验证捕获、DWM 裁剪、区域定位与 OCR，并为事务 E2E 提供可复现场景。
//!
//! 它是测试设施，不是产品功能：
//! - 只渲染合成文本，不含任何真实聊天内容。
//! - 只属于本进程，不与任何第三方程序交互。
//! - 提供像素级真值矩形，供区域定位精度评估。

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, LineTo, MoveToEx, SelectObject, SetBkMode, SetTextColor,
    CLEARTYPE_QUALITY, DEFAULT_CHARSET, DEFAULT_PITCH, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FF_DONTCARE, FW_NORMAL, HBRUSH, HDC, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, HWND_TOPMOST, PostQuitMessage,
    RegisterClassW, SetWindowTextW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WINDOW_EX_STYLE, WM_DESTROY,
    WM_PAINT, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

/// 主题。影响背景与文字颜色，用于验证定位对配色不敏感。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    fn window_bg(self) -> COLORREF {
        match self {
            Theme::Light => COLORREF(0x00F5F5F5),
            Theme::Dark => COLORREF(0x00201F1E),
        }
    }

    fn sidebar_bg(self) -> COLORREF {
        match self {
            Theme::Light => COLORREF(0x00E8E6E4),
            Theme::Dark => COLORREF(0x00171615),
        }
    }

    fn panel_bg(self) -> COLORREF {
        match self {
            Theme::Light => COLORREF(0x00FFFFFF),
            Theme::Dark => COLORREF(0x002B2A29),
        }
    }

    fn text(self) -> COLORREF {
        match self {
            Theme::Light => COLORREF(0x00202020),
            Theme::Dark => COLORREF(0x00F0F0F0),
        }
    }

    fn separator(self) -> COLORREF {
        match self {
            Theme::Light => COLORREF(0x00D0CFCE),
            Theme::Dark => COLORREF(0x00454443),
        }
    }
}

/// 合成场景。全部为固定文本，作为 OCR 真值。
#[derive(Debug, Clone)]
pub struct Scene {
    pub title: String,
    pub messages: Vec<String>,
    pub draft: String,
    pub theme: Theme,
}

impl Scene {
    pub fn sample(index: usize, theme: Theme) -> Self {
        const TITLES: [&str; 4] = ["项目协作群", "客户对接群", "张伟", "家人群"];
        const DRAFTS: [&str; 4] = [
            "方案已更新，请查收附件",
            "这个排期真的太离谱了",
            "明天上午十点开会",
            "记得带钥匙 OK",
        ];
        let i = index % TITLES.len();
        Self {
            title: TITLES[i].to_string(),
            messages: vec![
                "下午的评审准备好了吗".to_string(),
                "客户希望周五之前看到结论".to_string(),
                "我这边整理一下数据".to_string(),
            ],
            draft: DRAFTS[i].to_string(),
            theme,
        }
    }
}

/// 区域真值。客户区坐标，供定位精度评估使用。
#[derive(Debug, Clone, Copy)]
pub struct GroundTruth {
    pub chat_title: RECT,
    pub message_context: RECT,
    pub compose_input: RECT,
    pub separator_y: i32,
    pub sidebar_width: i32,
}

/// 布局参数。按客户区尺寸等比推导，模拟真实聊天软件的结构比例。
pub fn layout_for(client: RECT) -> GroundTruth {
    let w = client.right - client.left;
    let h = client.bottom - client.top;
    let sidebar = (w as f32 * 0.26).round() as i32;
    let title_h = (h as f32 * 0.09).round() as i32;
    let compose_h = (h as f32 * 0.24).round() as i32;
    let separator_y = h - compose_h;

    GroundTruth {
        chat_title: RECT {
            left: sidebar,
            top: 0,
            right: w,
            bottom: title_h,
        },
        message_context: RECT {
            left: sidebar,
            top: title_h,
            right: w,
            bottom: separator_y,
        },
        compose_input: RECT {
            left: sidebar,
            top: separator_y,
            right: w,
            bottom: h,
        },
        separator_y,
        sidebar_width: sidebar,
    }
}

struct WindowState {
    scene: Scene,
}

static SCENE: OnceLock<std::sync::Mutex<Option<WindowState>>> = OnceLock::new();

fn scene_slot() -> &'static std::sync::Mutex<Option<WindowState>> {
    SCENE.get_or_init(|| std::sync::Mutex::new(None))
}

/// 创建并显示测试聊天窗口。返回窗口句柄。
pub fn create_window(scene: Scene, width: i32, height: i32) -> Option<HWND> {
    *scene_slot().lock().ok()? = Some(WindowState {
        scene: scene.clone(),
    });

    // SAFETY: 标准窗口注册与创建流程，全部作用于本进程自己的窗口。
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class_name = w!("DangerChatTestChatWindow");

        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        // 重复注册返回 0，可忽略。
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("DangerChat 测试聊天窗口"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;

        let title = to_wide(&format!("DangerChat 测试 - {}", scene.title));
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
        let _ = ShowWindow(hwnd, SW_SHOW);
        // 置顶：在自动化测量环境中，终端会不断重申前台焦点；
        // 延迟测量需要的是窗口可见且不被遮挡，而不是"前台"本身。
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        Some(hwnd)
    }
}

/// 切换当前场景并重绘。用于模拟“同一窗口内切换会话”。
pub fn set_scene(hwnd: HWND, scene: Scene) {
    if let Ok(mut slot) = scene_slot().lock() {
        *slot = Some(WindowState {
            scene: scene.clone(),
        });
    }
    // SAFETY: 只操作本进程自己创建的窗口。
    unsafe {
        let title = to_wide(&format!("DangerChat 测试 - {}", scene.title));
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

pub fn destroy_window(hwnd: HWND) {
    // SAFETY: 销毁本进程自己创建的窗口。
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// 读取窗口客户区尺寸。
pub fn client_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: 传入本地 RECT，作用于本进程窗口。
    unsafe { GetClientRect(hwnd, &mut rect).ok()? };
    Some(rect)
}

/// 客户区在桌面坐标中的原点。用于把客户区真值精确换算到捕获帧坐标。
pub fn client_origin_on_screen(hwnd: HWND) -> Option<POINT> {
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY: ClientToScreen 只写入本地 POINT，作用于本进程窗口。
    let ok = unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut origin) };
    ok.as_bool().then_some(origin)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: 标准 WM_PAINT 处理，BeginPaint/EndPaint 成对调用。
            unsafe {
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hwnd, hdc);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 退出本窗口的消息循环。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: 其余消息交回默认处理。
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, hdc: HDC) {
    let Some(client) = client_rect(hwnd) else {
        return;
    };
    let Some(state) = scene_slot().lock().ok().and_then(|s| s.clone()) else {
        return;
    };
    let scene = state.scene;
    let gt = layout_for(client);
    let theme = scene.theme;

    // SAFETY: 所有 GDI 对象在使用后立即释放；绘制目标是本窗口的 DC。
    unsafe {
        fill(hdc, client, theme.window_bg());
        fill(
            hdc,
            RECT {
                left: 0,
                top: 0,
                right: gt.sidebar_width,
                bottom: client.bottom,
            },
            theme.sidebar_bg(),
        );
        fill(hdc, gt.chat_title, theme.panel_bg());
        fill(hdc, gt.message_context, theme.window_bg());
        fill(hdc, gt.compose_input, theme.panel_bg());

        // 侧栏条目块：提供稳定的结构特征，但不含文字。
        let entry_h = ((client.bottom - client.top) as f32 * 0.08).round() as i32;
        for i in 0..6 {
            let top = 8 + i * (entry_h + 6);
            if top + entry_h > client.bottom {
                break;
            }
            fill(
                hdc,
                RECT {
                    left: 8,
                    top,
                    right: gt.sidebar_width - 8,
                    bottom: top + entry_h,
                },
                theme.panel_bg(),
            );
        }

        // 输入区上方的分隔线：区域定位的主要结构锚点。
        draw_hline(
            hdc,
            gt.sidebar_width,
            client.right,
            gt.separator_y,
            theme.separator(),
        );
        // 标题栏下沿分隔线。
        draw_hline(
            hdc,
            gt.sidebar_width,
            client.right,
            gt.chat_title.bottom,
            theme.separator(),
        );
        // 侧栏与主区分隔线。
        draw_vline(hdc, gt.sidebar_width, 0, client.bottom, theme.separator());

        let title_font_h = ((gt.chat_title.bottom - gt.chat_title.top) as f32 * 0.5).round() as i32;
        let body_font_h = (title_font_h as f32 * 0.85).round().max(12.0) as i32;

        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, theme.text());

        draw_text(
            hdc,
            &scene.title,
            RECT {
                left: gt.chat_title.left + 16,
                top: gt.chat_title.top,
                right: gt.chat_title.right - 16,
                bottom: gt.chat_title.bottom,
            },
            title_font_h,
        );

        let msg_h = body_font_h * 2;
        for (i, message) in scene.messages.iter().enumerate() {
            let top = gt.message_context.top + 16 + (i as i32) * msg_h;
            if top + msg_h > gt.message_context.bottom {
                break;
            }
            draw_text(
                hdc,
                message,
                RECT {
                    left: gt.message_context.left + 16,
                    top,
                    right: gt.message_context.right - 16,
                    bottom: top + msg_h,
                },
                body_font_h,
            );
        }

        draw_text(
            hdc,
            &scene.draft,
            RECT {
                left: gt.compose_input.left + 16,
                top: gt.compose_input.top + 12,
                right: gt.compose_input.right - 16,
                bottom: gt.compose_input.top + 12 + body_font_h * 2,
            },
            body_font_h,
        );
    }
}

unsafe fn fill(hdc: HDC, rect: RECT, color: COLORREF) {
    // SAFETY: 创建的画刷在使用后立即删除。
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

unsafe fn draw_hline(hdc: HDC, x0: i32, x1: i32, y: i32, color: COLORREF) {
    // SAFETY: 画笔在使用后恢复并删除。
    unsafe {
        let pen = CreatePen(PS_SOLID, 1, color);
        let old = SelectObject(hdc, pen.into());
        let _ = MoveToEx(hdc, x0, y, None);
        let _ = LineTo(hdc, x1, y);
        SelectObject(hdc, old);
        let _ = DeleteObject(pen.into());
    }
}

unsafe fn draw_vline(hdc: HDC, x: i32, y0: i32, y1: i32, color: COLORREF) {
    // SAFETY: 同上。
    unsafe {
        let pen = CreatePen(PS_SOLID, 1, color);
        let old = SelectObject(hdc, pen.into());
        let _ = MoveToEx(hdc, x, y0, None);
        let _ = LineTo(hdc, x, y1);
        SelectObject(hdc, old);
        let _ = DeleteObject(pen.into());
    }
}

unsafe fn draw_text(hdc: HDC, text: &str, rect: RECT, height: i32) {
    // SAFETY: 字体在使用后恢复并删除；DrawTextW 写入本地宽字符缓冲。
    unsafe {
        let face = to_wide("Microsoft YaHei UI");
        let font = CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            PCWSTR(face.as_ptr()),
        );
        let old = SelectObject(hdc, font.into());
        let mut buffer = to_wide_no_nul(text);
        let mut r = rect;
        DrawTextW(
            hdc,
            &mut buffer,
            &mut r,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        SelectObject(hdc, old);
        let _ = DeleteObject(font.into());
    }
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn to_wide_no_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

impl Clone for WindowState {
    fn clone(&self) -> Self {
        Self {
            scene: self.scene.clone(),
        }
    }
}

/// 从命令行参数解析主题。
pub fn parse_theme(value: Option<&str>) -> Theme {
    value.and_then(Theme::parse).unwrap_or(Theme::Light)
}
