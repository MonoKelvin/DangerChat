//! 主程序装配（§5.9/§5.10）：数据目录 → 日志 → 配置 → 模型 → Intercept →
//! （window_watch 发现目标后 spawn Guard）→ 托盘。
//!
//! 顺序不可换：LogCenter 最先（后续步骤都有日志）；Intercept 在 Guard 前
//! （guard 消费它的 TriggerBus/Slot）。

use std::sync::Arc;

use tauri::Manager;

use dc_bridge::state::AppState;
use dc_bridge::stats::DailyStats;
use dc_core::logging::{LogCenter, LogOptions};
use dc_core::{ConfigCenter, ModelStore};
use dc_pipeline::intercept::{Intercept, InterceptConfig, InterceptDeps};
use dc_pipeline::{layout, ocr, sem};
use dc_sys::RealSys;

/// 系统默认数据目录：`%APPDATA%\<identifier>`。
fn default_data_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .expect("app_data_dir 解析失败（%APPDATA% 不可用）")
}

pub fn bootstrap(app: &tauri::AppHandle) -> Arc<AppState> {
    let default_dir = default_data_dir(app);
    let _ = std::fs::create_dir_all(&default_dir);
    // 自定义数据目录指针（默认目录下的 data_dir.txt）优先于系统默认
    let data_dir = dc_bridge::state::resolve_data_dir(default_dir.clone());
    let _ = std::fs::create_dir_all(&data_dir);

    // 1) 日志（最先：一切后续步骤可观测）
    let log_center = LogCenter::init(LogOptions {
        dir: data_dir.join("logs"),
        prefix: "danger".into(),
        level: "info".into(),
        retain_days: 7,
        max_total_mb: 64,
        console: cfg!(debug_assertions),
        local_offset_minutes: dc_sys::local_utc_offset_minutes(),
    })
    .map_err(|e| tracing::error!(error = ?e, "日志中心初始化失败（继续无文件日志）"))
    .ok();
    tracing::info!(dir = %data_dir.display(), "危信启动");

    // 2) 配置中心（模块 schema 注册 + finalize）
    let config = Arc::new(
        ConfigCenter::open(data_dir.join("config.toml")).expect("配置中心打开失败"),
    );
    {
        // intercept 的配置面是自由函数（含 target.process_name 等装配前需要的键）
        config
            .register_module("intercept", dc_pipeline::intercept_config_schema())
            .expect("intercept schema 注册失败");
        for module in [
            Box::new(layout::LayoutStage::new()) as Box<dyn dc_core::Module>,
            Box::new(ocr::OcrStage::new()),
            Box::new(sem::SemStage::empty()),
        ] {
            config
                .register_module(module.id(), module.config_schema())
                .expect("模块 schema 注册失败");
        }
        // guard 编排面（pipeline.* 前缀）
        config
            .register_module("pipeline", dc_pipeline::guard::config_schema())
            .expect("pipeline schema 注册失败");
        // 弹窗倒计时（bridge 自有配置）
        config
            .register_module(
                "alert",
                vec![
                    dc_core::ConfigField {
                        key: "alert.timeout_secs".into(),
                        ty: dc_core::ConfigType::Int { min: 3, max: 60 },
                        default: dc_core::ConfigValue::Int(10),
                        label: "弹窗倒计时".into(),
                        help: "超时后弹窗自动关闭（等同「关闭」）".into(),
                        group: "拦截与提示".into(),
                        owner: "alert".into(),
                    },
                    dc_core::ConfigField {
                        key: "alert.shake".into(),
                        ty: dc_core::ConfigType::Bool,
                        default: dc_core::ConfigValue::Bool(true),
                        label: "弹窗重点提示".into(),
                        help: "弹出时抖动窗口并闪烁边框".into(),
                        group: "拦截与提示".into(),
                        owner: "alert".into(),
                    },
                ],
            )
            .expect("alert schema 注册失败");
        config.finalize().expect("配置 finalize 失败");
    }

    // 3) 模型仓库（models/，权重 gitignore）
    let models = Arc::new(ModelStore::new(data_dir.join("models")));

    // 4) Intercept（钩子 + 前台监听；RAII guard 存在 AppState 生命周期外——
    //    由 Intercept 自身持有，进程退出随 Drop 卸载）
    let sys: Arc<dyn dc_sys::SysApi> = Arc::new(RealSys::new());
    // 配置从快照构建（曾硬编码 default → target.process_name 改动对钩子不生效）
    let deps = InterceptDeps::new(
        Arc::clone(&sys),
        InterceptConfig::from_snapshot(&config.snapshot()),
    );
    let intercept = Arc::new(Intercept::new(deps));
    // RAII guard 存栈上会在 bootstrap 返回时 Drop（钩子被卸载，M6 以来的隐性缺陷）。
    // 钩子必须活到进程结束——进程退出时 OS 自动解除，故 forget 释放所有权。
    std::mem::forget(intercept.start().expect("键盘钩子安装失败"));

    // 5) 组装 AppState
    let target = config
        .snapshot()
        .str_or("target.process_name", "WeChat.exe")
        .to_string();
    let state = Arc::new(AppState {
        sys,
        log_center,
        config: Arc::clone(&config),
        models,
        intercept: Arc::clone(&intercept),
        guard: std::sync::Mutex::new(None),
        target_process: std::sync::Mutex::new(target.clone()),
        stats: DailyStats::load(data_dir.join("stats.json")),
        countdown_secs: std::sync::Mutex::new(
            config.snapshot().i64_or("alert.timeout_secs", 10).max(1) as u64,
        ),
        data_dir: data_dir.clone(),
        default_data_dir: default_dir,
        // logo 基础色相（暖红）：前端主题色切换时经 set_tray_hue 覆盖
        tray_hue_deg: std::sync::atomic::AtomicU32::new(11),
    });

    // 6) 默认规则库/画像（首启空文件，sem 模块容忍缺失）
    let rules = data_dir.join("rules.toml");
    if !rules.is_file() {
        let _ = std::fs::write(
            &rules,
            r#"[[rule]]
pattern = "sb"
match = "word"
severity = "block"
applies_to = ["formal"]

[[rule]]
pattern = "(傻|沙)(比|逼|雕)"
match = "regex"
severity = "warn"
applies_to = ["formal"]

[[rule]]
pattern = "卧槽"
match = "substring"
severity = "warn"
applies_to = ["all"]
"#,
        );
    }
    let contacts = data_dir.join("contacts.toml");
    if !contacts.is_file() {
        let _ = std::fs::write(&contacts, "");
    }

    tracing::info!(target = %target, "装配完成（等待目标窗口发现）");
    state
}
