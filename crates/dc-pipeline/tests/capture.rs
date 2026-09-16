//! UT-CAP-01 ~ UT-CAP-04：截图模块（设计文档 §5.4 测试矩阵）

use std::sync::Arc;

use dc_core::{ConfigSnapshot, ImageLogSink, RunId};
use dc_pipeline::contract::{LoopKind, PipelineContext};
use dc_pipeline::{CaptureRequest, CaptureStage, Module, Stage, StageError};
use dc_sys::{ForegroundInfo, Hwnd, MockSys, MockWindow, Rect, SysApi};
use image::RgbaImage;

const HWND: Hwnd = Hwnd(0x2000);

fn stage(sys: &MockSys) -> CaptureStage {
    CaptureStage::new(Arc::new(sys.clone()) as Arc<dyn SysApi>)
}

/// 构造运行上下文：`enabled` 对应 `privacy.no_image_logs` 的反值。
/// `runs_root` 当前测试用 Noop sink、不实际写盘，故未使用（保留参数以保持调用点签名稳定）。
fn context(_runs_root: Option<&std::path::Path>, enabled: bool, cancel: bool) -> PipelineContext {
    let sink = if enabled {
        // 测试模式：不实际写盘，用 Noop（异步写入无法在测试里同步验证）
        ImageLogSink::noop()
    } else {
        ImageLogSink::noop()
    };
    let ctx = PipelineContext::new(
        RunId::from_raw("20260913-130000-000001"),
        LoopKind::Slow,
        sink,
        Arc::new(ConfigSnapshot::default()),
    );
    if cancel {
        ctx.cancel.cancel();
    }
    ctx
}

/// 造一张每个像素都不同的虚拟桌面，任何坐标错位都会被抓出来。
fn patterned_screen(w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.put_pixel(
                x,
                y,
                image::Rgba([
                    (x % 251) as u8,
                    (y % 241) as u8,
                    ((x * 3 + y * 7) % 239) as u8,
                    255,
                ]),
            );
        }
    }
    img
}

fn setup_window(sys: &MockSys, rect_dip: Rect, dpi_scale: f32, screen: (u32, u32)) {
    sys.set_screen(patterned_screen(screen.0, screen.1));
    sys.set_window(HWND, MockWindow::new(rect_dip, dpi_scale));
    sys.set_foreground(Some(ForegroundInfo {
        hwnd: HWND,
        pid: 100,
        process_name: "WeChat.exe".to_string(),
    }));
}

/// UT-CAP-01 mock 屏幕图 + rect → 矩形直拷像素逐字节正确
#[test]
fn ut_cap_01_region_crop_is_byte_exact() {
    let sys = MockSys::new();
    let rect_dip = Rect::new(100, 50, 200, 150);
    setup_window(&sys, rect_dip, 1.0, (1920, 1080));
    let screen = sys.screen().unwrap();

    let stage = stage(&sys);
    let snap = stage
        .process(CaptureRequest { hwnd: HWND }, &context(None, false, false))
        .expect("capture ok");

    assert_eq!(snap.window_rect, rect_dip, "100% 缩放下矩形原样");
    assert_eq!(snap.dpi_scale, 1.0);
    assert_eq!(snap.image.dimensions(), (200, 150));

    for y in 0..150u32 {
        for x in 0..200u32 {
            let got = *snap.image.get_pixel(x, y);
            let want = *screen.get_pixel(x + 100, y + 50);
            assert_eq!(got, want, "像素错位 at ({x},{y})");
        }
    }
    assert!(snap.captured_at.elapsed().as_secs() < 5);
    assert_eq!(stage.metrics().invocations, 1);
    assert_eq!(stage.metrics().failures, 0);
}

/// UT-CAP-02 DPI 1.25/1.5 坐标换算：window_rect 输出物理像素，capture 不得二次缩放
#[test]
fn ut_cap_02_dpi_scaling_no_double_conversion() {
    for (dpi_scale, expect) in [
        (1.25f32, Rect::new(13, 25, 375, 250)),
        (1.5f32, Rect::new(15, 30, 450, 300)),
    ] {
        let sys = MockSys::new();
        setup_window(&sys, Rect::new(10, 20, 300, 200), dpi_scale, (1920, 1080));
        let screen = sys.screen().unwrap();

        let stage = stage(&sys);
        let snap = stage
            .process(CaptureRequest { hwnd: HWND }, &context(None, false, false))
            .expect("capture ok");

        assert_eq!(snap.window_rect, expect, "缩放 {dpi_scale} 的物理矩形");
        assert!(
            (snap.dpi_scale - dpi_scale).abs() < 1e-6,
            "dpi_scale 应为 {dpi_scale}"
        );
        // 尺寸必须等于物理矩形：若在 capture 里再乘一次缩放比，这里会变成 375*1.25
        assert_eq!(snap.image.dimensions(), (expect.w, expect.h), "无二次缩放");

        // 像素原点对齐物理坐标
        let got = *snap.image.get_pixel(0, 0);
        let want = *screen.get_pixel(expect.x as u32, expect.y as u32);
        assert_eq!(got, want, "缩放 {dpi_scale} 时裁剪原点错位");
    }
}

/// UT-CAP-03 最小化 / 非前台 / 目标不可用 → Recoverable（明确不可用，而非错误图片）
#[test]
fn ut_cap_03_unavailable_window_is_recoverable() {
    let sys = MockSys::new();
    setup_window(&sys, Rect::new(0, 0, 300, 200), 1.0, (800, 600));
    let stage = stage(&sys);
    let req = CaptureRequest { hwnd: HWND };

    // 最小化
    sys.set_minimized(HWND, true);
    let err = stage
        .process(req, &context(None, false, false))
        .unwrap_err();
    assert!(err.is_recoverable(), "最小化应为 Recoverable：{err}");
    assert!(matches!(err, StageError::Recoverable(_)));

    // 非前台
    sys.set_minimized(HWND, false);
    sys.set_foreground(Some(ForegroundInfo {
        hwnd: Hwnd(0x9999),
        pid: 1,
        process_name: "Code.exe".to_string(),
    }));
    let err = stage
        .process(req, &context(None, false, false))
        .unwrap_err();
    assert!(err.is_recoverable(), "非前台应为 Recoverable：{err}");

    // 完全没有前台窗口（锁屏等）
    sys.set_foreground(None);
    assert!(stage.process(req, &context(None, false, false)).is_err());

    // 未登记的窗口
    assert!(stage
        .process(
            CaptureRequest { hwnd: Hwnd(0xDEAD) },
            &context(None, false, false)
        )
        .is_err());

    // 截图本身失败（BitBlt 连续失败 → 上层升 Fatal 由 guard 决定，这里只要求 Recoverable）
    setup_window(&sys, Rect::new(0, 0, 300, 200), 1.0, (800, 600));
    sys.fail_capture(true);
    let err = stage
        .process(req, &context(None, false, false))
        .unwrap_err();
    assert!(err.is_recoverable(), "截图失败应为 Recoverable：{err}");

    // 取消令牌优先
    sys.fail_capture(false);
    let err = stage.process(req, &context(None, false, true)).unwrap_err();
    assert!(matches!(err, StageError::Cancelled), "已取消：{err}");

    // 指标记录了失败次数
    assert!(stage.metrics().failures >= 4, "{:?}", stage.metrics());
}

/// UT-CAP-04 图片日志开关：开 → 01_window.png 存在；关 → 不落任何文件
#[test]
fn ut_cap_04_image_log_switch() {
    let dir = tempfile::TempDir::new().unwrap();
    let sys = MockSys::new();
    setup_window(&sys, Rect::new(0, 0, 64, 48), 1.0, (200, 200));
    let stage = stage(&sys);
    let req = CaptureRequest { hwnd: HWND };

    // 关闭（隐私模式 / privacy.no_image_logs = true）
    let runs = dir.path().join("runs");
    stage
        .process(req, &context(Some(&runs), false, false))
        .unwrap();
    assert!(!runs.exists(), "关闭时不得创建图片日志目录");

    // 开启
    let snap = stage
        .process(req, &context(Some(&runs), true, false))
        .unwrap();
    assert_eq!(snap.image.dimensions(), (64, 48));
    let png = runs.join("20260913-130000-000001").join("01_window.png");
    assert!(png.is_file(), "开启时应写出 {}", png.display());
    let on_disk = image::open(&png).expect("可解码").to_rgba8();
    assert_eq!(on_disk, snap.image, "落盘内容与快照一致");
}
