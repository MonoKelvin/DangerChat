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
    /// 场景管理器（scenes.toml 的唯一写方；判定侧另持快照，经 reload 同步）。
    pub scenarios: Mutex<dc_pipeline::sem::scenarios::ScenarioManager>,
    pub stats: DailyStats,
    /// 弹窗倒计时秒数（alert.timeout_secs，默认 10）。
    pub countdown_secs: Mutex<u64>,
    /// 数据目录（日志/配置/统计所在根）。
    pub data_dir: std::path::PathBuf,
    /// 系统默认数据目录（%APPDATA%\<identifier>；自定义目录指针存这里）。
    pub default_data_dir: std::path::PathBuf,
    /// 待清理的旧数据目录：`migrate_data_dir` 成功后写入，`delete_old_data_dir` 仅接受此路径。
    ///
    /// 为何需要它：`data_dir` 在进程启动时解析，运行期不变。迁移后指针已指向新目录，
    /// 但 `data_dir` 仍是旧目录 —— 此时若仅靠「不得等于 data_dir」来保护，
    /// 旧目录会被永久拒绝删除（迁移后的清理需求无法满足）。
    /// 改为白名单：只有「刚迁移走的那个目录」可删，安全边界反而更紧。
    pub pending_cleanup: Mutex<Option<std::path::PathBuf>>,
    /// 自助训练任务状态（start_training 线程写，training_status / 前端读）。
    pub training: Mutex<crate::dto::TrainingStatusDto>,
    /// 托盘图标主题色相（度数）：前端主题色切换时同步，图标按此色相着色。
    pub tray_hue_deg: std::sync::atomic::AtomicU32,
    /// 进程退出信号：置位后各后台线程（pump/window-watch）退出循环。
    ///
    /// 为何需要：这些线程是 `std::thread::spawn` 的**非分离线程**，
    /// Rust 主线程返回时会等待它们结束；而它们都是无退出条件的死循环，
    /// 导致 `app.exit(0)` 后事件循环虽结束、进程却仍存活（「托盘退出但后台还在」）。
    pub shutdown: std::sync::atomic::AtomicBool,
}

/// 自定义数据目录指针文件（存于默认目录；内容为自定义路径，一行）。
pub fn data_dir_pointer(default_data_dir: &std::path::Path) -> std::path::PathBuf {
    default_data_dir.join("data_dir.txt")
}

/// 解析生效数据目录：指针存在且指向有效目录 → 用之；否则默认目录。
pub fn resolve_data_dir(default_data_dir: std::path::PathBuf) -> std::path::PathBuf {
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
        self.target_process
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn set_target_process(&self, name: &str) {
        if let Ok(mut s) = self.target_process.lock() {
            *s = name.to_string();
        }
    }

    pub fn guard_running(&self) -> bool {
        self.guard.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// 请求退出：置位信号，后台线程在下一轮循环收敛。
    pub fn request_shutdown(&self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// 是否已请求退出。
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(std::sync::atomic::Ordering::SeqCst)
    }
}
