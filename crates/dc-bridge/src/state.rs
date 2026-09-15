//! AppState：bridge 的聚合根（`app.manage` 托管，commands 经 `State<AppState>` 访问）。

use std::sync::{Arc, Mutex};

use dc_core::{ConfigCenter, ModelStore};
use dc_pipeline::guard::Guard;
use dc_pipeline::intercept::Intercept;
use dc_sys::SysApi;

use crate::stats::DailyStats;

/// 退出原因：目标窗口消失后 Guard 被 window_watch 回收，带原因供状态事件展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardSlot {
    Running,
    /// 目标窗口未找到（发现循环进行中）。
    Discovering,
}

pub struct AppState {
    pub sys: Arc<dyn SysApi>,
    /// 日志中心句柄（clear_logs 用；init 在 bootstrap，Drop 由全局 tracing 管）。
    #[allow(dead_code)]
    pub log_center: Option<dc_core::logging::LogCenter>,
    pub config: Arc<ConfigCenter>,
    pub models: Arc<ModelStore>,
    pub intercept: Arc<Intercept>,
    /// Guard 句柄：window_watch 单线程独占写，commands 只读。
    pub guard: Mutex<Option<Guard>>,
    /// 目标进程名（config 快照的缓存，热更新经 set_config 生效）。
    pub target_process: Mutex<String>,
    pub stats: DailyStats,
    /// 弹窗倒计时秒数（alert.timeout_secs，默认 10）。
    pub countdown_secs: Mutex<u64>,
    /// 数据目录（日志/配置/统计所在根）。
    pub data_dir: std::path::PathBuf,
    /// 系统默认数据目录（%APPDATA%\<identifier>；自定义目录指针存这里）。
    pub default_data_dir: std::path::PathBuf,
    /// 托盘图标主题色相（度数）：前端主题色切换时同步，图标按此色相着色。
    pub tray_hue_deg: std::sync::atomic::AtomicU32,
}

/// 自定义数据目录指针文件（存于默认目录；内容为自定义路径，一行）。
pub fn data_dir_pointer(default_data_dir: &std::path::Path) -> std::path::PathBuf {
    default_data_dir.join("data_dir.txt")
}

/// 解析生效数据目录：指针存在且指向有效目录 → 用之；否则默认目录。
pub fn resolve_data_dir(
    default_data_dir: std::path::PathBuf,
) -> std::path::PathBuf {
    if let Ok(text) = std::fs::read_to_string(data_dir_pointer(&default_data_dir)) {
        let custom = std::path::PathBuf::from(text.trim());
        if !text.trim().is_empty() && custom.is_dir() {
            return custom;
        }
    }
    default_data_dir
}

impl AppState {
    pub fn target_process(&self) -> String {
        self.target_process.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_target_process(&self, name: &str) {
        if let Ok(mut s) = self.target_process.lock() {
            *s = name.to_string();
        }
    }

    pub fn guard_running(&self) -> bool {
        self.guard.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}
