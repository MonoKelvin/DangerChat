//! 测试夹具：把「真实世界」换成可脚本化对象，让流水线逻辑可确定性验证（§11.1）。
//!
//! 这不是「测试专用后门」——它是 §4 契约的自然产物：模块只依赖 `SysApi` / `Clock` 抽象，
//! 因此任何实现都能替换进来。dc-pipeline 之外（如 dc-bridge）同样可以用它写集成测试。

use std::sync::Arc;

use dc_sys::{ForegroundInfo, HookAction, Hwnd, KeyEvent, MockSys, SysApi};
use image::RgbaImage;

use crate::clock::{Clock, TestClock};
use crate::intercept::{Intercept, InterceptConfig, InterceptDeps, InterceptGuard};
use crate::verdict::Verdict;

/// 交互式夹具：按键、前台切换、判定投递全部同步可断言。
pub struct Harness {
    pub sys: MockSys,
    pub clock: Arc<TestClock>,
    pub intercept: Arc<Intercept>,
    /// 夹具自用的目标窗口句柄。
    pub hwnd: Hwnd,
    /// 已安装的钩子 + 前台监听守卫：保持存活，Drop 时卸载（与生产装配一致）。
    pub guard: InterceptGuard,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    pub fn new() -> Self {
        Self::with_config(InterceptConfig::default())
    }

    pub fn with_config(config: InterceptConfig) -> Self {
        Self::with_capacities(config, 8, 4)
    }

    /// 自定义两个有界队列容量（用于验证「队列满」分支：alert 容量 0 = 立即满 → fail-open）。
    pub fn with_capacities(config: InterceptConfig, trigger: usize, alert: usize) -> Self {
        let sys = MockSys::new();
        let clock = Arc::new(TestClock::new(1_000));
        let hwnd = Hwnd(0x1234);
        let deps = InterceptDeps::new(Arc::new(sys.clone()) as Arc<dyn SysApi>, config)
            .with_clock(Arc::clone(&clock) as Arc<dyn Clock>)
            .with_capacities(trigger, alert);
        let intercept = Arc::new(Intercept::new(deps));
        // 必须真的装上钩子与前台监听：否则 MockSys 的前台事件没有接收方，
        // 夹具会永远停留在 Suspended，测试就变成自欺欺人。
        let guard = intercept.start().expect("MockSys 安装钩子不会失败");
        Self {
            sys,
            clock,
            intercept,
            hwnd,
            guard,
        }
    }

    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    /// 推进时钟（让 TTL / allow-once / 去抖过期）。
    pub fn advance(&self, ms: u64) {
        self.clock.advance(ms);
    }

    // ---- 前台 ----

    pub fn target_process(&self) -> String {
        self.intercept.config().target_process.clone()
    }

    /// 模拟「目标程序成为前台」。
    pub fn target_foreground(&self) {
        self.sys.emit_foreground(ForegroundInfo {
            hwnd: self.hwnd,
            pid: 4242,
            process_name: self.target_process(),
        });
    }

    /// 模拟「切到别的程序」。
    pub fn other_foreground(&self, process: &str) {
        self.sys.emit_foreground(ForegroundInfo {
            hwnd: Hwnd(0x9999),
            pid: 777,
            process_name: process.to_string(),
        });
    }

    // ---- 按键 ----

    pub fn key_event(&self, vk: u16, ctrl: bool) -> KeyEvent {
        KeyEvent {
            vk,
            scan_code: 0,
            is_key_down: true,
            is_injected: false,
            ctrl,
            alt: false,
            shift: false,
        }
    }

    pub fn press(&self, vk: u16) -> HookAction {
        self.intercept.on_key(&self.key_event(vk, false))
    }

    pub fn press_ctrl(&self, vk: u16) -> HookAction {
        self.intercept.on_key(&self.key_event(vk, true))
    }

    pub fn release(&self, vk: u16) -> HookAction {
        let mut ev = self.key_event(vk, false);
        ev.is_key_down = false;
        self.intercept.on_key(&ev)
    }

    /// 依次敲入 ASCII 文本（每个字符都算内容修改键）。
    pub fn type_text(&self, text: &str) {
        for ch in text.chars() {
            if ch.is_ascii() {
                let _ = self.press(ch.to_ascii_uppercase() as u16);
            }
        }
    }

    pub fn enter(&self) -> HookAction {
        self.press(0x0D)
    }

    // ---- 判定 ----

    /// 投递一份判定并把纪元绑定到当前草稿纪元（模拟慢环/快环的产出）。
    pub fn publish(&self, verdict: Verdict) {
        let epoch = self.intercept.tracker().epoch();
        self.intercept
            .slot()
            .store(verdict.with_epoch(epoch), self.clock.now_ms());
    }

    /// 投递一份陈旧纪元判定（用于验证「纪元不等即失效」）。
    pub fn publish_with_epoch(&self, verdict: Verdict, epoch: u64) {
        self.intercept
            .slot()
            .store(verdict.with_epoch(epoch), self.clock.now_ms());
    }

    /// 只续期时间戳（心跳零推理续期，§2.3）。
    pub fn refresh_verdict(&self) -> bool {
        self.intercept.slot().refresh(self.clock.now_ms())
    }

    // ---- 屏幕 ----

    /// 造一张带可识别像素的虚拟桌面。
    pub fn set_screen(&self, width: u32, height: u32) {
        let mut img = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                img.put_pixel(
                    x,
                    y,
                    image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255]),
                );
            }
        }
        self.sys.set_screen(img);
    }

    /// 登记一个窗口（DIP 矩形 + 缩放比），并让它成为前台。
    pub fn register_window(&self, rect_dip: dc_sys::Rect, dpi_scale: f32) {
        self.sys
            .set_window(self.hwnd, dc_sys::MockWindow::new(rect_dip, dpi_scale));
        self.set_screen(1920, 1080);
        self.target_foreground();
    }
}

/// 造一个「危险」判定。
pub fn dangerous_verdict(reason: &str) -> Verdict {
    Verdict::block(reason)
}

// ---------------------------------------------------------------------------
// MockStage（M5 guard 测试）：可脚本化的 Stage 替身
// ---------------------------------------------------------------------------

use crate::contract::{Module, PipelineContext, Stage, StageError};
use dc_core::{MetricsRecorder, ModuleContext, ModuleError, ModuleMetrics};
use std::sync::Mutex as StdMutex;

/// 可脚本化的 Stage 替身：预设输出队列按序消费；失败注入（Recoverable/Fatal）；
/// 调用计数（含 init/shutdown 次数，验证模型生命周期）。
pub struct MockStage<I, O> {
    pub id: &'static str,
    /// 输出队列（消费到空后：重复最后一个输出，简化测试）。
    pub outputs: StdMutex<VecDeque<Result<O, StageError>>>,
    pub calls: std::sync::atomic::AtomicUsize,
    pub init_calls: std::sync::atomic::AtomicUsize,
    pub shutdown_calls: std::sync::atomic::AtomicUsize,
    /// process 收到的输入快照（断言用）。
    pub inputs: StdMutex<Vec<String>>,
    _marker: std::marker::PhantomData<fn(I) -> I>,
}

impl<I, O> MockStage<I, O>
where
    I: Clone + Send + Sync + 'static + std::fmt::Debug,
    O: Clone + Send + Sync + 'static,
{
    pub fn new(id: &'static str, outputs: Vec<Result<O, StageError>>) -> Self {
        Self {
            id,
            outputs: StdMutex::new(outputs.into()),
            calls: std::sync::atomic::AtomicUsize::new(0),
            init_calls: std::sync::atomic::AtomicUsize::new(0),
            shutdown_calls: std::sync::atomic::AtomicUsize::new(0),
            inputs: StdMutex::new(Vec::new()),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

use std::collections::VecDeque;

impl<I, O> Module for MockStage<I, O>
where
    I: Clone + Send + Sync + 'static + std::fmt::Debug,
    O: Clone + Send + Sync + 'static,
{
    fn id(&self) -> &'static str {
        self.id
    }
    fn config_schema(&self) -> Vec<dc_core::ConfigField> {
        Vec::new()
    }
    fn init(&mut self, _ctx: &ModuleContext) -> Result<(), ModuleError> {
        self.init_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    fn shutdown(&mut self) -> Result<(), ModuleError> {
        self.shutdown_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    fn metrics(&self) -> ModuleMetrics {
        MetricsRecorder::default().snapshot()
    }
}

impl<I, O> Stage for MockStage<I, O>
where
    I: Clone + Send + Sync + 'static + std::fmt::Debug,
    O: Clone + Send + Sync + 'static,
{
    type Input = I;
    type Output = O;

    fn process(
        &self,
        input: Self::Input,
        _ctx: &PipelineContext,
    ) -> Result<Self::Output, StageError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inputs
            .lock()
            .map(|mut v| v.push(format!("{input:?}")))
            .ok();
        let mut q = self
            .outputs
            .lock()
            .map_err(|_| StageError::Recoverable("mock 锁中毒".into()))?;
        if q.is_empty() {
            return Err(StageError::Recoverable(format!("{} 无预置输出", self.id)));
        }
        if q.len() == 1 {
            // 只剩一个：按其种类重复消费（简化长序列测试；StageError 不 Clone）
            match &q[0] {
                Ok(v) => Ok(v.clone()),
                Err(StageError::Recoverable(m)) => Err(StageError::Recoverable(m.clone())),
                Err(StageError::Fatal(m)) => Err(StageError::Fatal(m.clone())),
                Err(StageError::Cancelled) => Err(StageError::Cancelled),
            }
        } else {
            match q.pop_front() {
                Some(r) => r,
                None => unreachable!("len>1 已保证"),
            }
        }
    }
}
