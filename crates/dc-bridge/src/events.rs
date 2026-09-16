//! 事件名常量（§5.9 events 表）。

/// 弹窗：吞键瞬间 → dc-alert 显示。
pub const EVENT_ALERT: &str = "alert://blocked";
/// 状态迁移（Active/Suspended/Paused/目标发现）。
pub const EVENT_STATUS: &str = "guard://status";
/// 心跳统计（无消息原文）。
pub const EVENT_STATS: &str = "guard://stats";
/// models/ 清单变更（自助训练产出 / 手动放入新模型）。
pub const EVENT_MODELS: &str = "fs://models";
/// datasets/ 清单变更（uitag 新导出的训练数据）。
pub const EVENT_DATASETS: &str = "fs://datasets";
/// 训练任务状态迁移。
pub const EVENT_TRAINING: &str = "training://status";
