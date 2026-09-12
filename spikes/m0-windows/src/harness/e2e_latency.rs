//! M0-D 端到端延迟测量。
//!
//! 链路：模拟发送按键（t0）→ 显示器捕获 → 结构定位 → ROI 裁剪导出 →
//! 保活 Python OCR 进程识别标题与草稿 → 固定规则决策（t1）。
//!
//! 说明：
//! - 按键抑制的回调耗时由 M0-A 单独测量（约 0.1 ms 量级），
//!   此处以 t0 起算已包含等价成本。
//! - 尖峰阶段尚未有 UI；提示可见的渲染开销不计入，
//!   正式门槛复核在 M1 接入真实 UI 后重跑。
//! - OCR 通过保活子进程避免每事务冷启动模型。

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::harness::chat_window::{self, Scene, Theme};
use crate::harness::ocr_export::write_roi_png;
use crate::platform::capture::{capture_window_region, declare_dpi_awareness};
use crate::platform::clock::MonotonicClock;
use crate::platform::layout;
use crate::platform::window::foreground_snapshot;

/// 保活的 OCR 子进程。
struct OcrSidecar {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    stdin: Option<std::process::ChildStdin>,
}

impl OcrSidecar {
    fn spawn() -> Result<Self, String> {
        let repo = std::env::current_dir().map_err(|e| e.to_string())?;
        let worker_dir = repo.join("python").join("dc_worker");
        if !worker_dir.join("dc_worker").exists() {
            return Err(format!("找不到 Python Worker 目录: {}", worker_dir.display()));
        }

        let mut child = Command::new("uv")
            .args(["run", "--no-sync", "python", "-m", "dc_worker", "ocr-serve"])
            .current_dir(&worker_dir)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // CREATE_NO_WINDOW：子进程不创建控制台窗口，避免抢占前台焦点。
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| format!("无法启动 OCR 子进程: {e}"))?;

        let stdout = child.stdout.take().ok_or("无法取得子进程 stdout")?;
        let stdin = child.stdin.take().ok_or("无法取得子进程 stdin")?;
        let mut reader = BufReader::new(stdout);

        // 等待 READY。
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("读取 READY 失败: {e}"))?;
        if !line.trim().eq_ignore_ascii_case("READY") {
            return Err(format!("OCR 子进程未就绪: {line:?}"));
        }

        Ok(Self {
            child,
            reader,
            stdin: Some(stdin),
        })
    }

    /// 识别一张 PNG，返回文本。
    fn recognize(&mut self, path: &PathBuf) -> Result<String, String> {
        let stdin = self.stdin.as_mut().ok_or("stdin 已关闭")?;
        writeln!(stdin, "{}", path.display()).map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;

        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| format!("读取 OCR 结果失败: {e}"))?;
        let text = line.trim_end_matches(['\r', '\n']).to_string();
        if let Some(err) = text.strip_prefix("__ERROR__ ") {
            return Err(format!("OCR 失败: {err}"));
        }
        Ok(text)
    }

    fn shutdown(&mut self) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "SHUTDOWN");
            let _ = stdin.flush();
        }
        // 优雅等待最多 5 秒，超时强制终止。
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 一轮完整的分析链。
///
/// 说明：延迟测量不要求测试窗口处于前台（自动化环境中终端会不断重申焦点），
/// 而是依赖置顶显示 + 定位成功作为"捕获到正确像素"的校验。
/// 真实产品的前台触发语义由领域层单测与 M0-A 回放覆盖。
fn analyze_once(
    sidecar: &mut OcrSidecar,
    clock: &MonotonicClock,
    workdir: &PathBuf,
    iteration: usize,
) -> Result<u64, String> {
    let t0 = clock.now_nanos();

    let snapshot = foreground_snapshot().ok_or("没有前台窗口")?;
    let frame = capture_window_region(snapshot.instance.root_hwnd, snapshot.client, clock)
        .map_err(|e| format!("捕获失败: {e:?}"))?;
    let detected = layout::detect(
        &frame.pixels,
        frame.width as i32,
        frame.height as i32,
        frame.stride as usize,
    )
    .map_err(|e| format!("定位失败: {e:?}"))?;

    let title_png = workdir.join(format!("t{iteration}.png"));
    let draft_png = workdir.join(format!("d{iteration}.png"));
    write_roi_png(&title_png, &frame, inset(detected.chat_title, 4, 4))
        .map_err(|e| e.to_string())?;
    let compose = detected.compose_input;
    let draft_roi = windows::Win32::Foundation::RECT {
        left: compose.left,
        top: compose.top + 2,
        right: compose.right,
        bottom: compose.top + (compose.bottom - compose.top) / 2,
    };
    write_roi_png(&draft_png, &frame, draft_roi).map_err(|e| e.to_string())?;

    let _title = sidecar.recognize(&title_png)?;
    let draft = sidecar.recognize(&draft_png)?;

    // 固定规则决策：命中风险词则确认，否则放行。仅为延迟测量，不是产品策略。
    let risky = ["离谱", "受不了", "辞职"].iter().any(|w| draft.contains(w));
    let _decision = if risky { "CONFIRM" } else { "ALLOW_PERMIT" };

    let _ = std::fs::remove_file(&title_png);
    let _ = std::fs::remove_file(&draft_png);

    Ok(clock.now_nanos().saturating_sub(t0))
}

fn inset(rect: windows::Win32::Foundation::RECT, dx: i32, dy: i32) -> windows::Win32::Foundation::RECT {
    windows::Win32::Foundation::RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right - dx,
        bottom: rect.bottom - dy,
    }
}

/// 运行端到端延迟测量。
pub fn run(rounds: usize) -> Result<Vec<u64>, String> {
    declare_dpi_awareness();
    let clock = MonotonicClock::new();

    let repo = std::env::current_dir().map_err(|e| e.to_string())?;
    let workdir = repo.join("artifacts").join("m0-e2e");
    std::fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;

    // 测试窗口保持前台，场景轮换以覆盖不同文本。
    let scene = Scene::sample(0, Theme::Light);
    let Some(hwnd) = chat_window::create_window(scene, 1280, 820) else {
        return Err("无法创建测试窗口".to_string());
    };
    std::thread::sleep(Duration::from_millis(350));
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(150));

    let mut sidecar = OcrSidecar::spawn()?;

    let mut durations = Vec::with_capacity(rounds);
    let mut errors: Vec<String> = Vec::new();
    for i in 0..rounds {
        // 轮换场景文本，模拟不同草稿。
        chat_window::set_scene(
            hwnd,
            Scene::sample(i, if i % 2 == 0 { Theme::Light } else { Theme::Dark }),
        );
        crate::harness::layout_eval::pump_messages_public(Duration::from_millis(60));

        match analyze_once(&mut sidecar, &clock, &workdir, i) {
            Ok(nanos) => durations.push(nanos),
            Err(e) => errors.push(format!("第 {i} 轮: {e}")),
        }
    }

    sidecar.shutdown();
    chat_window::destroy_window(hwnd);
    crate::harness::layout_eval::pump_messages_public(Duration::from_millis(100));

    if !errors.is_empty() {
        let sample: Vec<String> = errors.iter().take(3).cloned().collect();
        return Err(format!(
            "{}/{} 轮分析失败，示例：{}",
            errors.len(),
            rounds,
            sample.join("；")
        ));
    }
    Ok(durations)
}
