//! 时间工具。
//!
//! 刻意不引入 `chrono` / `time`：本 crate 只需要「按日期滚动日志 + 文件名日期解析」
//! 这一个能力，自实现 civil 日历换算即可（Howard Hinnant 的 days_from_civil 及其逆算法）。
//!
//! 时区：核心不假设时区，一律由调用方提供 UTC 偏移（分钟）。真机偏移由 `dc-sys`
//! 经 Win32 `GetTimeZoneInformation` 取得后注入（见 `LogOptions::local_offset_minutes`）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前 Unix 时间戳（秒）。系统时间早于纪元时返回 0。
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 自 civil 日期到 days-since-epoch（1970-01-01 为 0）。
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // Mar=0 … Feb=11
    let doy = (153 * mp as i64 + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// days-since-epoch 到 civil 日期 `(year, month, day)`。
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix 秒 → `YYYY-MM-DD`（按给定 UTC 偏移，单位分钟）。
pub fn date_string(unix_secs: i64, offset_minutes: i32) -> String {
    let shifted = unix_secs + offset_minutes as i64 * 60;
    let days = shifted.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Unix 秒 → `YYYYMMDD-HHMMSS`（按给定 UTC 偏移，单位分钟）。用于 run id。
pub fn compact_datetime(unix_secs: i64, offset_minutes: i32) -> String {
    let shifted = unix_secs + offset_minutes as i64 * 60;
    let days = shifted.div_euclid(86_400);
    let secs = shifted.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// 解析 `YYYY-MM-DD` 为 days-since-epoch；格式非法返回 `None`。
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse::<i64>().ok()
    };
    let y = num(0..4)?;
    let m = num(5..7)?;
    let d = num(8..10)?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m as u32, d as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for days in [-25_000i64, -1, 0, 1, 19_000, 21_000, 40_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "days={days}");
        }
    }

    #[test]
    fn known_date() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        // 2026-09-13 00:00:00 UTC
        let midnight = days_from_civil(2026, 9, 13) * 86_400;
        assert_eq!(date_string(midnight, 0), "2026-09-13");
        assert_eq!(parse_date("2026-09-13"), Some(days_from_civil(2026, 9, 13)));
        assert_eq!(compact_datetime(midnight, 0), "20260913-000000");
    }

    #[test]
    fn offset_shifts_date() {
        // UTC 2026-09-12 20:00 → UTC+8 已是 09-13
        let t = days_from_civil(2026, 9, 12) * 86_400 + 20 * 3600;
        assert_eq!(date_string(t, 480), "2026-09-13");
        assert_eq!(date_string(t, 0), "2026-09-12");
    }

    #[test]
    fn parse_rejects_garbage() {
        for bad in ["", "2026/09/13", "2026-9-13", "abcd-ef-gh"] {
            assert_eq!(parse_date(bad), None, "bad={bad}");
        }
    }
}
