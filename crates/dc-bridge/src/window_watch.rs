//! 目标窗口发现状态机（§5.8 装配前置）：**单线程独占 Guard spawn 权**，
//! 结构上消除 respawn 竞态。
//!
//! 状态循环：
//! ```text
//! Discovering ──(find_window 命中)──► spawn Guard ──► Watching
//!     ▲                                                    │
//!     └─────(窗口消失/最小化 2 次确认)──── Guard::stop ───┘
//! ```
//! 2s 轮询；前台事件不直接驱动（发现是主动查询语义，避免与 watch_foreground
//! 的多消费者语义纠缠）。目标进程名从 AppState 读（set_config 热更即时生效）。

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use dc_sys::SysApi;

use crate::events;
use crate::state::AppState;

const POLL: Duration = Duration::from_secs(2);

/// 退出信号轮询片段（毫秒）：把 POLL 切成小段，保证退出及时响应（最坏 EXIT_POLL_MS）。
const EXIT_POLL_MS: u64 = 200;
/// 窗口连续缺失多少次才判定消失（防目标程序瞬时无响应导致的抖动 respawn）。
const MISS_CONFIRM: u32 = 2;

pub fn spawn(app: AppHandle) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("window-watch".into())
        .spawn(move || watch_loop(app))
        .expect("window-watch spawn")
}

fn watch_loop(app: AppHandle) {
    // state 引用不能跨 loop 持有（State<'_, _> 借用 app）——每轮重取
    let mut misses: u32 = 0;
    loop {
        let state = app.state::<Arc<AppState>>();
        let process = state.target_process();
        let found = SysApi::find_window_by_process(state.sys.as_ref(), &process);

        match (state.guard_running(), found) {
            (false, Some(hwnd)) => {
                // 发现 → spawn Guard（此线程唯一写者）
                let ctx_state = Arc::clone(&state);
                match dc_pipeline::guard::Guard::spawn(
                    &state.intercept,
                    Arc::clone(&state.sys),
                    move || module_ctx(&ctx_state),
                    state.config.snapshot(),
                    hwnd,
                ) {
                    Ok(guard) => {
                        misses = 0;
                        if let Ok(mut slot) = state.guard.lock() {
                            *slot = Some(guard);
                        }
                        tracing::info!(process = %process, "目标窗口已发现，Guard 已启动");
                        let _ = app.emit(events::EVENT_STATUS, crate::commands::status_of(&state));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Guard 启动失败（下轮重试）");
                    }
                }
            }
            (true, Some(_)) => {
                misses = 0;
            }
            (true, None) => {
                misses += 1;
                if misses >= MISS_CONFIRM {
                    misses = 0;
                    // 消失 → stop Guard 回发现态
                    if let Ok(mut slot) = state.guard.lock() {
                        if let Some(mut g) = slot.take() {
                            g.stop();
                        }
                    }
                    tracing::info!(process = %process, "目标窗口消失，Guard 已停止（进入发现态）");
                    let _ = app.emit(events::EVENT_STATUS, crate::commands::status_of(&state));
                }
            }
            (false, None) => {}
        }

        // 退出信号：托盘「退出」后收敛本线程。
        // 检查置于 sleep 前，并把 2s 等待切成 200ms 片段 —— 否则最坏要等满 2s
        // 才会看到信号，而主线程正在等本线程结束（非分离线程会卡住进程终止）。
        for _ in 0..(POLL.as_millis() as u64 / EXIT_POLL_MS) {
            if state.is_shutting_down() {
                tracing::info!("window-watch 收到退出信号，线程结束");
                return;
            }
            std::thread::sleep(Duration::from_millis(EXIT_POLL_MS));
        }
    }
}

/// ModuleContext 工厂：models 根 = data_dir/models。
fn module_ctx(state: &AppState) -> dc_core::ModuleContext {
    dc_core::ModuleContext {
        config: state.config.snapshot(),
        models: Arc::clone(&state.models),
        log_dir: state.data_dir.join("logs"),
    }
}
