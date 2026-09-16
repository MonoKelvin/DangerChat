//! # dc-bridge — 前后端通信层（设计文档 §5.9）
//!
//! - [`state::AppState`]：Intercept / Guard / ConfigCenter / ModelStore / 统计的聚合根，
//!   `app.manage()` 托管；
//! - [`bootstrap`]：bootstrap.json 管理（数据目录指针）；
//! - [`dto`]：跨进程契约（Verdict → DTO 映射；draft_text 只进弹窗事件，日志永不落原文）；
//! - [`commands`]：11 条 `#[tauri::command]`；
//! - [`events`]：事件名常量与载荷；
//! - [`pump`]：AlertBus 泵线程（Show → Tauri 事件 + 弹窗定位显示；Action → 代转）；
//! - [`stats`]：今日拦截数按日累加器（stats.json 落盘）；
//! - [`window_watch`]：目标窗口发现状态机（唯一持有 Guard spawn 权）；
//! - [`dir_watch`]：models/ 与 datasets/ 清单监听（自助训练数据流）。
//!
//! 依赖 tauri 2（command 宏 + Emitter trait）；tauri 不进 dc-core/pipeline/sys。

pub mod bootstrap;
pub mod commands;
pub mod dir_watch;
pub mod dto;
pub mod events;
pub mod pump;
pub mod state;
pub mod stats;
pub mod window_watch;
