//! 单调时钟。截止时间、新鲜度与延迟统计一律使用它，不使用系统墙钟。

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// 基于 QPC 的单调纳秒时钟。
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    frequency: i64,
}

impl MonotonicClock {
    pub fn new() -> Self {
        let mut frequency = 0i64;
        // SAFETY: QueryPerformanceFrequency 写入我们提供的栈变量；
        // 文档保证在 Windows XP 及以上始终成功。
        unsafe {
            let _ = QueryPerformanceFrequency(&mut frequency);
        }
        debug_assert!(frequency > 0);
        Self {
            frequency: frequency.max(1),
        }
    }

    /// 自系统启动以来的单调纳秒数。
    pub fn now_nanos(&self) -> u64 {
        let mut counter = 0i64;
        // SAFETY: 同上，仅写入栈变量。
        unsafe {
            let _ = QueryPerformanceCounter(&mut counter);
        }
        let counter = counter.max(0) as u128;
        let nanos = counter * 1_000_000_000u128 / self.frequency as u128;
        nanos as u64
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}
