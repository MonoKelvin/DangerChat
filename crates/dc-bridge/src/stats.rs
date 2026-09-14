//! 今日拦截统计：按日累加器 + stats.json 落盘（跨日滚动）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DayCounters {
    /// YYYY-MM-DD（本地日期）。
    date: String,
    blocked: u64,
    alert_failed: u64,
}

/// 进程内统计句柄（原子累加 + 惰性跨日滚动 + 落盘）。
pub struct DailyStats {
    inner: std::sync::Mutex<DayCounters>,
    path: PathBuf,
}

impl DailyStats {
    /// 从 stats.json 恢复（不存在/损坏 → 空计数）。
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let inner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let s = Self {
            inner: std::sync::Mutex::new(inner),
            path,
        };
        s.roll_if_new_day(&today());
        s
    }

    pub fn blocked(&self) -> u64 {
        self.roll_if_new_day(&today());
        self.inner.lock().map(|c| c.blocked).unwrap_or(0)
    }

    pub fn alert_failed(&self) -> u64 {
        self.roll_if_new_day(&today());
        self.inner.lock().map(|c| c.alert_failed).unwrap_or(0)
    }

    /// 拦截 +1（弹窗 Show 成功路径）。
    pub fn record_block(&self) {
        self.roll_if_new_day(&today());
        if let Ok(mut c) = self.inner.lock() {
            c.blocked += 1;
        }
        self.persist();
    }

    /// 弹窗展示失败 +1（fail-open 落空）。
    pub fn record_alert_failed(&self) {
        self.roll_if_new_day(&today());
        if let Ok(mut c) = self.inner.lock() {
            c.alert_failed += 1;
        }
        self.persist();
    }

    fn roll_if_new_day(&self, today: &str) {
        if let Ok(mut c) = self.inner.lock() {
            if c.date != today {
                c.date = today.to_string();
                c.blocked = 0;
                c.alert_failed = 0;
            }
        }
    }

    fn persist(&self) {
        if let Ok(c) = self.inner.lock() {
            let tmp = self.path.with_extension("json.tmp");
            if let Ok(text) = serde_json::to_string(&*c) {
                if std::fs::write(&tmp, text).is_ok() {
                    let _ = std::fs::rename(&tmp, &self.path);
                }
            }
        }
    }
}

/// 本地日期 YYYY-MM-DD（dc_sys 平台 API，不引 chrono）。
fn today() -> String {
    let (y, m, d) = dc_sys::local_ymd();
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_persistence() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("stats.json");
        {
            let s = DailyStats::load(&path);
            assert_eq!(s.blocked(), 0);
            s.record_block();
            s.record_block();
            s.record_alert_failed();
            assert_eq!(s.blocked(), 2);
            assert_eq!(s.alert_failed(), 1);
        }
        // 重新加载：持久化恢复
        let s = DailyStats::load(&path);
        assert_eq!(s.blocked(), 2);
        assert_eq!(s.alert_failed(), 1);
        // 落盘的是合法 JSON 且含日期键
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"date\""), "{text}");
        assert!(text.contains("\"blocked\":2"), "{text}");
    }

    #[test]
    fn corrupt_file_starts_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("stats.json");
        std::fs::write(&path, "not json").unwrap();
        let s = DailyStats::load(&path);
        assert_eq!(s.blocked(), 0);
    }
}
