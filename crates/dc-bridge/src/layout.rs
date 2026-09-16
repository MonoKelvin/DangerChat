//! 数据目录布局：数据目录内所有配置路径与类型化文档句柄的唯一来源。
//!
//! 设计要点（§5.1「配置路径唯一来源」的落地）：
//! - **路径推导收口**：rules/contacts/scenes/stats 等文件不再由 `mctx.log_dir.parent()`
//!   或 `state.data_dir.join(常量)` 在多处各自反推，一律经本结构取用，消除推导规则漂移
//!   （sem 用 log_dir 反推、bridge 用 data_dir 直拼曾是两条不一致的路径）。
//! - **类型化句柄**：contacts/rules/scenes 这类「一个 JSON 就是一个类型」的文档，
//!   经 [`JsonStore`] 提供读-改-写 + 原子落盘 + 损坏自愈，调用方不再手写
//!   `read_to_string → 解析 → 序列化 → write` 样板，也不再各自实现不一致的容错。
//!
//! 与 [`crate::bootstrap`] 的分工：bootstrap.json 固定在 Roaming 目录、不随数据目录迁移，
//! 由 bootstrap 模块自持；本结构只管**数据目录内部**的布局。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dc_core::config_doc::JsonStore;
use dc_core::ConfigError;

/// 数据目录内一份配置文档的句柄集合：路径 + 类型化读写。
///
/// 三个字段对应 sem 热重载所需的三份数据（规则库 / 联系人画像 / 场景表）。
pub struct SemDocs {
    pub rules_path: PathBuf,
    pub contacts_path: PathBuf,
    pub scenes_path: PathBuf,
    pub rules: Arc<JsonStore<dc_pipeline::sem::doc::RuleDoc>>,
    pub contacts: Arc<JsonStore<dc_pipeline::sem::doc::ContactDoc>>,
}

/// 数据目录布局。装配期一次性构建，运行期只读。
pub struct DataLayout {
    data_dir: PathBuf,
    rules: Arc<JsonStore<dc_pipeline::sem::doc::RuleDoc>>,
    contacts: Arc<JsonStore<dc_pipeline::sem::doc::ContactDoc>>,
}

impl DataLayout {
    /// 打开数据目录下的全部配置文档。目录不存在时逐文档容错（JsonStore 内部已兜底）。
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let data_dir = data_dir.into();
        Ok(Self {
            rules: open_doc(&data_dir, dc_core::paths::RULES_JSON)?,
            contacts: open_doc(&data_dir, dc_core::paths::CONTACTS_JSON)?,
            data_dir,
        })
    }

    /// 数据目录根（logs/ models/ datasets/ 的父目录）。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    pub fn datasets_dir(&self) -> PathBuf {
        self.data_dir.join("datasets")
    }

    /// 规则库文档（rules.json 的类型化句柄）。
    pub fn rules(&self) -> Arc<JsonStore<dc_pipeline::sem::doc::RuleDoc>> {
        Arc::clone(&self.rules)
    }

    /// 联系人画像文档（contacts.json 的类型化句柄）。
    pub fn contacts(&self) -> Arc<JsonStore<dc_pipeline::sem::doc::ContactDoc>> {
        Arc::clone(&self.contacts)
    }

    /// 场景表路径（scenes.json 由 sem 的 ScenarioManager 自持读写，此处仅提供路径）。
    pub fn scenes_path(&self) -> PathBuf {
        self.data_dir.join(dc_core::paths::SCENES_JSON)
    }

    /// 统计文件路径（stats.json 由 DailyStats 自持读写，此处仅提供路径）。
    pub fn stats_path(&self) -> PathBuf {
        self.data_dir.join(dc_core::paths::STATS_JSON)
    }

    /// sem 热重载三件套：三份路径 + 其中两份的类型化句柄。
    ///
    /// 收口此前散落的「bridge 拼三路径 → guard 传字符串 → sem 再读盘」链路：
    /// 路径只在此产生一次，调用方（commands）直接取用，不再手写 `.join(常量)`。
    pub fn sem_docs(&self) -> SemDocs {
        SemDocs {
            rules_path: self.rules.path().to_path_buf(),
            contacts_path: self.contacts.path().to_path_buf(),
            scenes_path: self.scenes_path(),
            rules: Arc::clone(&self.rules),
            contacts: Arc::clone(&self.contacts),
        }
    }
}

/// 打开一份 `<root>/<file>` 的类型化文档（文件名常量在 dc-core::paths 单一来源）。
fn open_doc<T>(root: &Path, file: &str) -> Result<Arc<JsonStore<T>>, ConfigError>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Default + Clone + Send + Sync + 'static,
{
    let stem = file.strip_suffix(".json").unwrap_or(file);
    Ok(Arc::new(JsonStore::open(stem, root.join(file))?))
}
