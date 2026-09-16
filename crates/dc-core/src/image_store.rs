//! 循环目录图片存储（调试模式专用，§5.1 图片日志增强版）。
//!
//! 设计要点：
//! - **循环覆盖**：1 ~ limit 编号目录，达到上限后从头开始，删除旧目录后写入。
//! - **配置驱动**：`debug.image_dirs_limit` 调小时删除超出部分。
//! - **游标持久化**：存 `bootstrap.json` 的 `images_cursor` 字段，与数据目录指针同文件。
//! - **异步写入**：后台线程 + 有界队列（满则丢弃旧帧），避免阻塞流水线。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;

/// 异步图片保存请求。
#[derive(Debug, Clone)]
pub struct SaveRequest {
    pub dir_index: u32,
    pub name: String,
    pub image: Arc<RgbaImage>,
}

/// 循环目录图片存储管理器。
pub struct RollingImageStore {
    root: PathBuf,
    limit: u32,
    cursor: Arc<AtomicU64>,
    tx: Option<Sender<SaveRequest>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl RollingImageStore {
    /// 创建存储管理器并启动后台写入线程。
    ///
    /// - `root`：图片根目录（如 `logs/images`）
    /// - `limit`：循环目录数量上限（10-100）
    /// - `initial_cursor`：初始游标（1-based，从 bootstrap.json 读取）
    pub fn new(root: impl Into<PathBuf>, limit: u32, initial_cursor: u64) -> Self {
        let root = root.into();
        let cursor = Arc::new(AtomicU64::new(initial_cursor.max(1)));
        let (tx, rx) = crossbeam_channel::bounded(32); // 有界队列，满则丢弃

        // 后台线程：从队列取图片编码并落盘
        let worker_root = root.clone();
        let worker = std::thread::Builder::new()
            .name("image-store".into())
            .spawn(move || {
                Self::worker_loop(worker_root, rx);
            })
            .expect("启动图片存储线程失败");

        Self {
            root,
            limit,
            cursor,
            tx: Some(tx),
            worker: Some(worker),
        }
    }

    /// 获取下一个目录索引（1-based），并自动循环。
    pub fn next_index(&self) -> u32 {
        let current = self.cursor.fetch_add(1, Ordering::SeqCst);
        ((current - 1) % self.limit as u64 + 1) as u32
    }

    /// 当前游标值（用于持久化到 bootstrap.json）。
    pub fn current_index(&self) -> u32 {
        let cursor = self.cursor.load(Ordering::SeqCst);
        ((cursor - 1) % self.limit as u64 + 1) as u32
    }

    /// 异步保存图片（非阻塞，队列满时静默丢弃）。
    pub fn save_async(&self, dir_index: u32, name: impl Into<String>, image: Arc<RgbaImage>) {
        let req = SaveRequest {
            dir_index,
            name: name.into(),
            image,
        };
        // try_send：队列满时立即返回错误，不阻塞
        if let Some(tx) = &self.tx {
            if let Err(e) = tx.try_send(req) {
                tracing::warn!(error = ?e, "图片保存队列已满，丢弃当前帧");
            }
        }
    }

    /// 调整上限：删除超出部分的目录。
    pub fn adjust_limit(&mut self, new_limit: u32) -> Result<(), std::io::Error> {
        if new_limit >= self.limit {
            self.limit = new_limit;
            return Ok(());
        }
        // 调小：删除 (new_limit+1) ~ old_limit 的目录
        for i in (new_limit + 1)..=self.limit {
            let dir = self.root.join(i.to_string());
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
                tracing::info!(path = %dir.display(), "已删除超限目录");
            }
        }
        self.limit = new_limit;
        Ok(())
    }

    /// 后台线程主循环：取请求 → 编码 → 写盘。
    fn worker_loop(root: PathBuf, rx: Receiver<SaveRequest>) {
        while let Ok(req) = rx.recv() {
            if let Err(e) = Self::save_sync(&root, &req) {
                tracing::warn!(
                    error = %e,
                    dir = req.dir_index,
                    name = req.name,
                    "图片保存失败"
                );
            }
        }
    }

    /// 同步保存单张图片（后台线程调用）。
    fn save_sync(root: &Path, req: &SaveRequest) -> Result<(), ImageStoreError> {
        // `name` 允许带子目录（`<run_id>/01_window`，见 §5.1 的 `runs/<run_id>/` 约定），
        // 此时父目录必须一并创建 —— `RgbaImage::save` 不会建中间目录，缺了就是
        // 一次 ENOENT 静默失败：目录建出来了、图却没有。
        let path = root.join(req.dir_index.to_string()).join(&req.name);
        let path = if req.name.ends_with(".png") {
            path
        } else {
            path.with_extension("png")
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ImageStoreError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        req.image.save(&path).map_err(|e| match e {
            image::ImageError::IoError(source) => ImageStoreError::Io { path, source },
            other => ImageStoreError::Encode(format!("{}: {}", req.name, other)),
        })?;
        Ok(())
    }
}

impl Drop for RollingImageStore {
    fn drop(&mut self) {
        // 显式关闭 channel：take() 丢弃 Sender，让 rx.recv() 返回断开错误
        drop(self.tx.take());
        // 再 join worker 线程（此时 worker_loop 的 recv() 已收到断开信号）
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImageStoreError {
    #[error("image encode failed: {0}")]
    Encode(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
