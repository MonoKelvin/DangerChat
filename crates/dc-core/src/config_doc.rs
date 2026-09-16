//! JSON 配置文档原语与配置管理器（§5.1 扩展）。
//!
//! - [`JsonStore<T>`]：一个 JSON 配置文档的全部通用能力——类型化读写、原子落盘、损坏自愈。
//! - [`ConfigManager`]：配置文档注册表，**配置路径的唯一来源**。
//!   扩展方式：新增配置文件 = `register::<NewDoc>("new")`（得到 `new.json`）；
//!   新增配置内容 = 新 schema 项（[`crate::config::ConfigField`]）或新 JSON 条目。
//! - [`Document`]：管理器的统一清单接口（[`crate::config::ConfigCenter`] 同样实现它），
//!   于是全部配置文件可被统一列举、重载、落盘。
//!
//! 与 [`crate::config::ConfigCenter`] 的分工：`ConfigCenter` 管**schema 驱动的键值配置**
//! （前端设置页据此渲染表单、变更要广播）；`JsonStore<T>` 管**整体类型化文档**
//! （规则库 / 对象画像 / 场景表 / 启动引导配置这类「一个 JSON 就是一个类型」的文件）。

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::{ConfigCenter, ConfigError, RecoveryRecord};
use crate::json_file;

/// 管理器统一清单接口：所有配置文件都能被列举、重载、落盘、查询恢复记录。
pub trait Document: Send + Sync {
    /// 文档名（= 文件名主干，如 `rules` / `config` / `bootstrap`）。
    fn name(&self) -> &str;
    fn path(&self) -> &Path;
    /// 原子落盘当前内存值。
    fn flush(&self) -> Result<(), ConfigError>;
    /// 重读磁盘（损坏 → 备份 + 回落默认值）。
    fn reload(&self) -> Result<(), ConfigError>;
    /// 加载期间的恢复记录（空 = 配置健康）。
    fn recoveries(&self) -> Vec<RecoveryRecord>;
}

struct StoreInner<T> {
    name: String,
    path: PathBuf,
    value: RwLock<T>,
    /// 打开时文件是否存在（首启播种判据）；首次成功落盘后置 false。
    fresh: AtomicBool,
    recoveries: Mutex<Vec<RecoveryRecord>>,
}

/// 类型化 JSON 配置文档。多线程共享用 `Arc<JsonStore<T>>`。
pub struct JsonStore<T> {
    inner: Arc<StoreInner<T>>,
}

impl<T> Clone for JsonStore<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> JsonStore<T>
where
    T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
{
    /// 打开（或创建）文档。文件缺失 → 默认值；文件损坏 → 备份 + 默认值重建。
    pub fn open(name: impl Into<String>, path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let name = name.into();
        let path = path.into();
        let (value, existed, recoveries) = load_or_recover(&path)?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                name,
                path,
                value: RwLock::new(value),
                fresh: AtomicBool::new(!existed),
                recoveries: Mutex::new(recoveries),
            }),
        })
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// 是否「本次打开时文件还不存在」——首启播种的判据（落盘一次后即为 false）。
    pub fn is_fresh(&self) -> bool {
        self.inner.fresh.load(Ordering::Relaxed)
    }

    /// 读当前值（克隆）。
    pub fn read(&self) -> T {
        self.inner.value.read().expect("value poisoned").clone()
    }

    /// 在只读借用下取值，避免整份克隆。
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let value = self.inner.value.read().expect("value poisoned");
        f(&value)
    }

    /// 读-改-写：落盘成功才换入内存值（写失败内存保持不变，不产生「磁盘没写进去但内存已变」的中间态）。
    pub fn write(&self, f: impl FnOnce(&mut T)) -> Result<(), ConfigError> {
        let mut guard = self.inner.value.write().expect("value poisoned");
        let mut draft = guard.clone();
        f(&mut draft);
        let bytes = json_file::serialize_pretty(&draft)?;
        json_file::write_atomic(&self.inner.path, &bytes)?;
        *guard = draft;
        self.inner.fresh.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// 整体替换（等价于 `write(|slot| *slot = value)`）。
    pub fn replace(&self, value: T) -> Result<(), ConfigError> {
        self.write(|slot| *slot = value)
    }

    /// 加载期间的恢复记录。
    pub fn recoveries(&self) -> Vec<RecoveryRecord> {
        self.inner
            .recoveries
            .lock()
            .expect("recoveries poisoned")
            .clone()
    }
}

impl<T> Document for JsonStore<T>
where
    T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        JsonStore::name(self)
    }

    fn path(&self) -> &Path {
        JsonStore::path(self)
    }

    fn flush(&self) -> Result<(), ConfigError> {
        let bytes = {
            let value = self.inner.value.read().expect("value poisoned");
            json_file::serialize_pretty(&*value)?
        };
        json_file::write_atomic(&self.inner.path, &bytes)?;
        self.inner.fresh.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn reload(&self) -> Result<(), ConfigError> {
        let (value, _, recoveries) = load_or_recover::<T>(&self.inner.path)?;
        *self.inner.value.write().expect("value poisoned") = value;
        if !recoveries.is_empty() {
            self.inner
                .recoveries
                .lock()
                .expect("recoveries poisoned")
                .extend(recoveries);
        }
        Ok(())
    }

    fn recoveries(&self) -> Vec<RecoveryRecord> {
        JsonStore::recoveries(self)
    }
}

/// 读文档；缺失 → 默认值，损坏 → 备份 + 默认值。
fn load_or_recover<T: DeserializeOwned + Default>(
    path: &Path,
) -> Result<(T, bool, Vec<RecoveryRecord>), ConfigError> {
    let Some(text) = json_file::read_optional_text(path)? else {
        return Ok((T::default(), false, Vec::new()));
    };
    match serde_json::from_str::<T>(&text) {
        Ok(value) => Ok((value, true, Vec::new())),
        Err(e) => {
            let record = json_file::backup_broken(path, &e.to_string())?;
            Ok((T::default(), true, vec![record]))
        }
    }
}

/// 配置管理器：数据目录内所有配置文档的注册表与**路径唯一来源**。
///
/// 新增一个配置文件只需一行注册，不必再手写加载/保存/容错。
pub struct ConfigManager {
    root: PathBuf,
    /// 已注册文档（按注册顺序，`documents()` 与 `flush_all`/`reload_all` 据此遍历）。
    documents: RwLock<Vec<Arc<dyn Document>>>,
    /// 已注册文档名（重名即装配期错误，panic）。
    names: Mutex<BTreeSet<String>>,
    /// 类型化句柄，供 `get::<T>(name)` 取回。
    typed: RwLock<BTreeMap<String, Arc<dyn Any + Send + Sync>>>,
    config: Mutex<Option<Arc<ConfigCenter>>>,
}

impl ConfigManager {
    /// 以 `root` 为配置根目录（通常是数据目录；不存在则创建）。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        if let Err(e) = std::fs::create_dir_all(&root) {
            tracing::warn!(dir = %root.display(), error = %e, "配置目录创建失败（后续落盘会重试）");
        }
        Self {
            root,
            documents: RwLock::new(Vec::new()),
            names: Mutex::new(BTreeSet::new()),
            typed: RwLock::new(BTreeMap::new()),
            config: Mutex::new(None),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 文档路径的唯一推导规则：`<root>/<name>.json`（`name` 含 `/` 即分组到子目录）。
    pub fn document_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.json"))
    }

    /// 注册一个类型化 JSON 文档；返回句柄由调用方持有。
    pub fn register<T>(&self, name: &str) -> Result<Arc<JsonStore<T>>, ConfigError>
    where
        T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static,
    {
        self.reserve(name);
        let store = Arc::new(JsonStore::open(name, self.document_path(name))?);
        self.typed.write().expect("typed poisoned").insert(
            name.to_string(),
            Arc::clone(&store.inner) as Arc<dyn Any + Send + Sync>,
        );
        self.documents
            .write()
            .expect("documents poisoned")
            .push(Arc::clone(&store) as Arc<dyn Document>);
        Ok(store)
    }

    /// 注册 schema 驱动的键值文档（`config.json`）。
    pub fn register_config_center(&self) -> Result<Arc<ConfigCenter>, ConfigError> {
        self.reserve("config");
        let center = Arc::new(ConfigCenter::open(self.document_path("config"))?);
        *self.config.lock().expect("config poisoned") = Some(Arc::clone(&center));
        self.documents
            .write()
            .expect("documents poisoned")
            .push(Arc::clone(&center) as Arc<dyn Document>);
        Ok(center)
    }

    /// 按名字取类型化句柄（名字未注册或类型不符 → `None`）。
    pub fn get<T: 'static + Send + Sync>(&self, name: &str) -> Option<Arc<JsonStore<T>>> {
        let raw = self
            .typed
            .read()
            .expect("typed poisoned")
            .get(name)
            .cloned()?;
        let inner = raw.downcast::<StoreInner<T>>().ok()?;
        Some(Arc::new(JsonStore { inner }))
    }

    pub fn config_center(&self) -> Option<Arc<ConfigCenter>> {
        self.config.lock().expect("config poisoned").clone()
    }

    /// 全部已注册文档（按注册顺序）。
    pub fn documents(&self) -> Vec<Arc<dyn Document>> {
        self.documents.read().expect("documents poisoned").clone()
    }

    /// 聚合各文档的恢复记录（启动时统一上报，避免静默重置配置）。
    pub fn recoveries(&self) -> Vec<(String, RecoveryRecord)> {
        self.documents()
            .iter()
            .flat_map(|d| {
                let name = d.name().to_string();
                d.recoveries().into_iter().map(move |r| (name.clone(), r))
            })
            .collect()
    }

    /// 把所有文档的内存值原子落盘。
    pub fn flush_all(&self) -> Result<(), ConfigError> {
        for doc in self.documents() {
            doc.flush()?;
        }
        Ok(())
    }

    /// 重新读取所有文档（外部改过文件后用）。
    pub fn reload_all(&self) -> Result<(), ConfigError> {
        for doc in self.documents() {
            doc.reload()?;
        }
        Ok(())
    }

    /// 占名；重名是装配期错误（与 §5.1 的配置键抢注同处置：开发期就该暴露）。
    fn reserve(&self, name: &str) {
        let mut names = self.names.lock().expect("names poisoned");
        assert!(
            names.insert(name.to_string()),
            "配置文档 `{name}` 已注册，不得重复注册（§5.1）"
        );
    }
}
