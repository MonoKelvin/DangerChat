//! JSON 配置文档的共用读写原语：原子落盘 + 损坏自愈。
//!
//! 抽出来给 [`crate::config::ConfigCenter`] 与 [`crate::config_doc::JsonStore`] 共用，
//! 保证数据目录里所有配置文件走**同一套**写入与容错路径（§5.1）。

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::config::{ConfigError, RecoveryRecord};
use crate::time;

/// 与目标同目录的临时文件名（`<path>.tmp`）。同目录才能用 rename 原子替换。
pub(crate) fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// 损坏文件的备份名（`<path>.broken-<unix_ts>`），保留现场供用户排查。
pub(crate) fn broken_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(format!(".broken-{}", time::unix_now()));
    PathBuf::from(s)
}

/// pretty JSON + 末尾换行（末尾换行使文件在编辑器/版本控制下更规整）。
pub(crate) fn serialize_pretty<T: Serialize>(value: &T) -> Result<Vec<u8>, ConfigError> {
    let mut text =
        serde_json::to_string_pretty(value).map_err(|e| ConfigError::Parse(e.to_string()))?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// 原子写：父目录 → 写 tmp → fsync → rename 替换。
///
/// Windows 上目标文件被别的进程打开（用户在编辑器里开着 `rules.json`）时
/// `MoveFileEx(REPLACE_EXISTING)` 会返回共享冲突/拒绝访问，故带短暂退避重试；
/// 任何失败路径都清掉 tmp 残留，不留半成品文件。
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    let tmp = tmp_path(path);
    let written = fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()));
    if let Err(source) = written {
        let _ = fs::remove_file(&tmp);
        return Err(ConfigError::Io {
            path: tmp,
            source,
        });
    }
    if let Err(source) = rename_with_retry(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// 短暂退避重试 rename（见 [`write_atomic`] 的说明）。
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut delay = Duration::from_millis(20);
    for attempt in 0..5 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 4 && is_transient(&e) => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("循环内必然返回")
}

/// 目标被占用导致的瞬时错误（Windows：ERROR_ACCESS_DENIED=5 / ERROR_SHARING_VIOLATION=32）。
#[cfg(windows)]
fn is_transient(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(32))
}

#[cfg(not(windows))]
fn is_transient(_: &std::io::Error) -> bool {
    false
}

/// 读文件文本；文件不存在返回 `None`（调用方按「用默认值」处理）。
pub(crate) fn read_optional_text(path: &Path) -> Result<Option<String>, ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// 配置损坏时的统一处置：把现场改名成 `.broken-<ts>` 留档，返回恢复记录。
pub(crate) fn backup_broken(path: &Path, reason: &str) -> Result<RecoveryRecord, ConfigError> {
    let backup = broken_path(path);
    fs::rename(path, &backup).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    tracing::warn!(
        path = %path.display(),
        backup = %backup.display(),
        reason,
        "配置文件损坏，已备份并以默认值重建"
    );
    Ok(RecoveryRecord {
        backup,
        reason: reason.to_string(),
    })
}
