//! 配置文件名常量（单一来源，避免硬编码散落各处）。
//!
//! 所有配置文件的文件名在此集中定义，其他模块通过此模块引用。
//! 路径构造由各层负责（data_dir 由 bootstrap 管理），此处仅提供文件名。

/// 启动配置文件名（数据目录指针，存放在 Roaming 目录）
pub const BOOTSTRAP_JSON: &str = "bootstrap.json";

/// 主配置文件名
pub const CONFIG_JSON: &str = "config.json";

/// 规则库文件名
pub const RULES_JSON: &str = "rules.json";

/// 联系人画像文件名
pub const CONTACTS_JSON: &str = "contacts.json";

/// 场景管理文件名
pub const SCENES_JSON: &str = "scenes.json";

/// 统计数据文件名
pub const STATS_JSON: &str = "stats.json";
