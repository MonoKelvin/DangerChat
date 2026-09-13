//! 可注入时间源。
//!
//! 为什么不用 `Instant::now()` 直接调用：判定有效期（TTL）、allow-once 超时、前台去抖都是
//! **时间敏感**逻辑，只有把时间源注入进来，单元测试才能确定性地快进时间，而不是靠 `sleep` 赌时序
//! （§11.1「Win32 一律 MockSys」的同一思路：外部世界全部可替换）。
//!
//! 生产环境用 [`MonoClock`]（进程启动为基准的单调时钟，不受系统时间调整影响）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 单调毫秒时钟。
pub trait Clock: Send + Sync + 'static {
    /// 自时钟基准起的毫秒数。仅用于比较，绝对值无意义。
    fn now_ms(&self) -> u64;
}

/// 生产用单调时钟。
#[derive(Debug)]
pub struct MonoClock {
    base: Instant,
}

impl MonoClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
        }
    }
}

impl Default for MonoClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonoClock {
    fn now_ms(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }
}

/// 测试用手动时钟：只有显式 [`TestClock::advance`] 才会前进。
#[derive(Debug, Default)]
pub struct TestClock {
    now: AtomicU64,
}

impl TestClock {
    pub fn new(start_ms: u64) -> Self {
        Self {
            now: AtomicU64::new(start_ms),
        }
    }

    /// 推进时间（等价于让 TTL / 超时 / 去抖真正过期）。
    pub fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set(&self, ms: u64) {
        self.now.store(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}
