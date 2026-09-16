//! 日志中心（FR-SYS-02/03/05，设计文档 §5.1）。
//!
//! - 文件日志：`{dir}/{prefix}-YYYY-MM-DD.log`，按日滚动；保留天数与总量双上限清理。
//! - 图片日志：[`ImageLogSink`] 把某一次分析过程的图写入 `{dir}/runs/<run_id>/`
//!   （`01_window.png` / `02_layout.png` / `03_ocr.png`）；隐私模式或未开启图片日志时为零副作用 Noop。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

use crate::time;

/// 日志中心启动参数。默认值即 §5.1 约定的默认配置。
#[derive(Debug, Clone)]
pub struct LogOptions {
    pub dir: PathBuf,
    /// 日志文件前缀，产物形如 `danger-2026-09-13.log`。
    pub prefix: String,
    /// tracing 级别过滤（`error`/`warn`/`info`/`debug`/`trace`，也可用 `module=level` 语法）。
    pub level: String,
    pub retain_days: u32,
    pub max_total_mb: u64,
    /// 是否同时输出到控制台（发布版静默启动时应关闭）。
    pub console: bool,
    /// 本地时区相对 UTC 的偏移（分钟），用于日志文件按**本地**日期滚动。
    /// 真机取值由 `dc-sys` 经 Win32 取得后注入（见 `dc_sys::local_utc_offset_minutes`）。
    pub local_offset_minutes: i32,
}

impl LogOptions {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            prefix: "danger".to_string(),
            level: "info".to_string(),
            retain_days: 7,
            max_total_mb: 200,
            console: true,
            local_offset_minutes: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("log level `{0}` invalid")]
    BadLevel(String),
}

/// 清理结果，供日志与前端「一键清空/统计」展示。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PruneReport {
    pub removed: Vec<PathBuf>,
    pub freed_bytes: u64,
    pub remaining_files: usize,
    pub remaining_bytes: u64,
}

/// 按日滚动的日志写入器（实现 [`MakeWriter`]，每个事件 clone 一份共享底层文件句柄）。
#[derive(Clone)]
pub struct DailyLogWriter {
    inner: Arc<WriterInner>,
}

struct WriterInner {
    dir: PathBuf,
    prefix: String,
    offset_minutes: i32,
    state: Mutex<Option<OpenFile>>,
}

struct OpenFile {
    date: String,
    path: PathBuf,
    file: fs::File,
}

impl DailyLogWriter {
    pub fn new(dir: impl Into<PathBuf>, prefix: impl Into<String>, offset_minutes: i32) -> Self {
        Self {
            inner: Arc::new(WriterInner {
                dir: dir.into(),
                prefix: prefix.into(),
                offset_minutes,
                state: Mutex::new(None),
            }),
        }
    }

    /// 当天日志文件路径（不保证存在）。
    pub fn current_path(&self) -> PathBuf {
        let date = time::date_string(time::unix_now(), self.inner.offset_minutes);
        log_file_path(&self.inner.dir, &self.inner.prefix, &date)
    }

    fn open_for(&self, date: &str) -> io::Result<OpenFile> {
        fs::create_dir_all(&self.inner.dir)?;
        let path = log_file_path(&self.inner.dir, &self.inner.prefix, date);
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(OpenFile {
            date: date.to_string(),
            path,
            file,
        })
    }

    /// 刷新并关闭当前文件；返回被写入的路径（若本次进程从未写过则为 `None`）。
    pub fn flush(&self) -> io::Result<Option<PathBuf>> {
        let mut guard = self.inner.state.lock().expect("log state poisoned");
        match guard.as_mut() {
            Some(open) => {
                open.file.flush()?;
                Ok(Some(open.path.clone()))
            }
            None => Ok(None),
        }
    }
}

impl Write for DailyLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let today = time::date_string(time::unix_now(), self.inner.offset_minutes);
        let mut guard = self.inner.state.lock().expect("log state poisoned");
        let need_rotate = match guard.as_ref() {
            Some(open) => open.date != today,
            None => true,
        };
        if need_rotate {
            *guard = Some(self.open_for(&today)?);
        }
        match guard.as_mut() {
            Some(open) => open.file.write(buf),
            None => Err(io::Error::other("log writer not initialised")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self.inner.state.lock().expect("log state poisoned");
        match guard.as_mut() {
            Some(open) => open.file.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for DailyLogWriter {
    type Writer = DailyLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn log_file_path(dir: &Path, prefix: &str, date: &str) -> PathBuf {
    dir.join(format!("{prefix}-{date}.log"))
}

/// 从文件名解析日志日期（`danger-2026-09-13.log` → `2026-09-13`）。
fn parse_log_date(file_name: &str, prefix: &str) -> Option<String> {
    let stem = file_name.strip_prefix(prefix)?.strip_prefix('-')?;
    let stem = stem.strip_suffix(".log")?;
    time::parse_date(stem)?;
    Some(stem.to_string())
}

/// 日志占用的目录（图片日志根，§5.1 `logs/runs/`）。
pub fn runs_root(log_dir: &Path) -> PathBuf {
    log_dir.join("runs")
}

/// 日志中心。持有 tracing 订阅者与日志文件句柄，`Drop` 时刷新。
pub struct LogCenter {
    opts: LogOptions,
    writer: DailyLogWriter,
    installed: bool,
}

impl LogCenter {
    /// 安装全局 tracing 订阅者。重复调用（如测试中多次初始化）不会 panic，只是不重复安装。
    pub fn init(opts: LogOptions) -> Result<Self, LogError> {
        let writer = DailyLogWriter::new(&opts.dir, &opts.prefix, opts.local_offset_minutes);
        fs::create_dir_all(&opts.dir).map_err(|source| LogError::Io {
            path: opts.dir.clone(),
            source,
        })?;
        let filter = tracing_subscriber::EnvFilter::try_new(&opts.level)
            .map_err(|_| LogError::BadLevel(opts.level.clone()))?;

        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(writer.clone());

        let installed = if opts.console {
            let console_layer = tracing_subscriber::fmt::layer()
                .with_ansi(true)
                .with_target(false)
                .with_writer(io::stdout);
            tracing_subscriber::registry()
                .with(filter)
                .with(file_layer)
                .with(console_layer)
                .try_init()
                .is_ok()
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(file_layer)
                .try_init()
                .is_ok()
        };

        Ok(Self {
            opts,
            writer,
            installed,
        })
    }

    pub fn options(&self) -> &LogOptions {
        &self.opts
    }

    pub fn dir(&self) -> &Path {
        &self.opts.dir
    }

    /// 本次进程是否真的安装成功（测试或已初始化环境里可能为 false）。
    pub fn is_installed(&self) -> bool {
        self.installed
    }

    /// 图片日志根目录 `logs/runs`。
    pub fn runs_dir(&self) -> PathBuf {
        runs_root(&self.opts.dir)
    }

    /// 为某一轮分析创建图片日志句柄。从 AppState.image_store 传入。
    pub fn image_sink(&self, store: Option<Arc<std::sync::Mutex<crate::image_store::RollingImageStore>>>) -> ImageLogSink {
        match store {
            Some(s) => ImageLogSink::from_store(s),
            None => ImageLogSink::noop(),
        }
    }

    /// 按保留天数与总量上限清理日志文件。
    pub fn prune(&self) -> Result<PruneReport, LogError> {
        prune_logs(
            &self.opts.dir,
            &self.opts.prefix,
            self.opts.retain_days,
            self.opts.max_total_mb,
            self.opts.local_offset_minutes,
        )
    }

    /// 一键清空日志目录内容（FR-SYS-05）：删除日志文件与 `runs/` 下全部图片，保留目录本身。
    pub fn clear_all(&self) -> Result<PruneReport, LogError> {
        let mut report = PruneReport::default();
        let entries = match fs::read_dir(&self.opts.dir) {
            Ok(e) => e,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(report),
            Err(source) => {
                return Err(LogError::Io {
                    path: self.opts.dir.clone(),
                    source,
                })
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path == self.runs_dir() {
                    collect_and_remove(&path, &mut report)?;
                }
                continue;
            }
            remove_file_counted(&path, &mut report)?;
        }
        Ok(report)
    }
}

impl Drop for LogCenter {
    fn drop(&mut self) {
        let _ = self.writer.flush();
    }
}

fn remove_file_counted(path: &Path, report: &mut PruneReport) -> Result<(), LogError> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    match fs::remove_file(path) {
        Ok(()) => {
            report.freed_bytes += size;
            report.removed.push(path.to_path_buf());
            Ok(())
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LogError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn collect_and_remove(dir: &Path, report: &mut PruneReport) -> Result<(), LogError> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(LogError::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_and_remove(&path, report)?;
            let _ = fs::remove_dir(&path);
        } else {
            remove_file_counted(&path, report)?;
        }
    }
    Ok(())
}

/// 清理日志目录：先按保留天数删过期文件，再按总量上限从最旧删起。
///
/// **永不删除「今天」的日志文件**（进程正持有其写句柄）。
pub fn prune_logs(
    dir: &Path,
    prefix: &str,
    retain_days: u32,
    max_total_mb: u64,
    offset_minutes: i32,
) -> Result<PruneReport, LogError> {
    let mut report = PruneReport::default();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(source) => {
            return Err(LogError::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
    };

    let today_days = time::unix_now().div_euclid(86_400) + (offset_minutes / 1440) as i64;
    let cutoff = today_days - retain_days as i64;
    let today = time::date_string(time::unix_now(), offset_minutes);

    // (日期, 路径, 体积)，按日期升序
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(date) = parse_log_date(name, prefix) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        files.push((date, path, size));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    // 1) 超出保留天数
    let mut kept: Vec<(String, PathBuf, u64)> = Vec::new();
    for (date, path, size) in files {
        let expired = date != today && time::parse_date(&date).map(|d| d < cutoff).unwrap_or(false);
        if expired {
            remove_file_counted(&path, &mut report)?;
        } else {
            kept.push((date, path, size));
        }
    }

    // 2) 总量上限（从最旧删起，跳过今天）
    let limit = max_total_mb * 1024 * 1024;
    let mut total: u64 = kept.iter().map(|(_, _, s)| *s).sum();
    let mut overflow: Vec<PathBuf> = Vec::new();
    for (date, path, size) in &kept {
        if total <= limit {
            break;
        }
        if date == &today {
            continue;
        }
        overflow.push(path.clone());
        total = total.saturating_sub(*size);
    }
    for path in &overflow {
        remove_file_counted(path, &mut report)?;
    }
    kept.retain(|(_, p, _)| !overflow.contains(p));

    report.remaining_files = kept.len();
    report.remaining_bytes = kept.iter().map(|(_, _, s)| *s).sum();
    Ok(report)
}

// ---------------------------------------------------------------------------
// 识别过程图片日志（FR-SYS-03/05）
// ---------------------------------------------------------------------------

/// 一轮分析的唯一 id，贯穿该轮图片日志目录名（§4 `PipelineContext.run_id`）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(String);

impl RunId {
    /// 形如 `20260913-132145-000007`：本地时间 + 自增序号，保证同秒内多次分析目录不冲突。
    pub fn new(seq: u64, offset_minutes: i32) -> Self {
        Self(format!(
            "{}-{seq:06}",
            time::compact_datetime(time::unix_now(), offset_minutes)
        ))
    }

    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImageLogError {
    #[error("image encode failed: {0}")]
    Encode(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// 图片日志句柄。`Noop` 与 `Store` 的调用侧代码完全一致，隐私开关只影响构造（§5.1）。
#[derive(Clone)]
pub enum ImageLogSink {
    /// 不落盘（隐私模式或 `debug.save_images = false`）。
    Noop,
    /// 异步保存到循环目录（调试模式）。
    Store {
        store: Arc<std::sync::Mutex<crate::image_store::RollingImageStore>>,
        dir_index: u32,
    },
}

impl ImageLogSink {
    /// 从 RollingImageStore 创建（调试模式开启时）。
    pub fn from_store(store: Arc<std::sync::Mutex<crate::image_store::RollingImageStore>>) -> Self {
        let dir_index = store.lock().expect("image store lock").next_index();
        ImageLogSink::Store { store, dir_index }
    }

    /// 创建 Noop（调试模式关闭时）。
    pub fn noop() -> Self {
        ImageLogSink::Noop
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self, ImageLogSink::Store { .. })
    }

    /// 保存一张过程图。`name` 形如 `01_window`（自动补 `.png`）。
    /// 异步非阻塞：图片编码与写盘在后台线程完成。
    pub fn save(&self, name: &str, image: &image::RgbaImage) -> Result<(), ImageLogError> {
        match self {
            ImageLogSink::Noop => Ok(()),
            ImageLogSink::Store { store, dir_index } => {
                let store = store.lock().expect("image store lock");
                store.save_async(*dir_index, name, Arc::new(image.clone()));
                Ok(())
            }
        }
    }
}
