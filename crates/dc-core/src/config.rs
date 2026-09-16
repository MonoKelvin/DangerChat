//! 配置中心（FR-SYS-01，设计文档 §5.1）。
//!
//! 设计要点：
//! - **schema 驱动**：每个模块用 [`ConfigField`] 声明自己的配置项（键、类型、范围、默认值、说明），
//!   前端只依赖 schema 渲染表单，新增配置项零前端改动（§5.10）。
//! - **原子持久化**：`写 tmp → fsync → rename` 替换 `config.json`，并发写不丢配置（UT-CORE-04）。
//! - **损坏自愈**：配置文件解析失败时备份为 `.broken-<ts>` 并以默认值重建（UT-CORE-03）。
//! - **快照语义**：运行中的流水线只读 [`ConfigSnapshot`]，每轮开始取一次，运行中配置变更不影响本轮（§4）。
//!
//! 点分键（`guard.send_key`）落盘为嵌套 JSON 对象（`{"guard":{"send_key":"enter"}}`），
//! 与实际文件形态一一对应，便于用户手改。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

use crate::config_doc;
use crate::json_file;

/// 配置项类型与校验约束。序列化后随 schema 下发前端，用于选择控件与做本地校验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigType {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
    /// 单行文本，`max_len` 为 0 表示不限制。
    Text {
        max_len: usize,
    },
    /// 枚举（单选项），`options[0]` 即默认值语义上的首选。
    Enum {
        options: Vec<String>,
    },
    /// 字符串列表（违禁词库噪声词表一类）。
    StrList,
    /// 文件/目录路径，界面上渲染为选择器。
    Path,
}

impl ConfigType {
    /// 校验并规范化取值；`Int` 接受整值浮点，`Float` 接受整数（JSON 只有一种数字，
    /// 手写 `1` 与 `1.0` 都可能落到任一分支）。
    pub fn normalize(&self, value: &ConfigValue) -> Result<ConfigValue, String> {
        match (self, value) {
            (ConfigType::Bool, ConfigValue::Bool(b)) => Ok(ConfigValue::Bool(*b)),
            (ConfigType::Int { min, max }, ConfigValue::Int(i)) => {
                if i < min || i > max {
                    return Err(format!("out of range [{min}, {max}]: {i}"));
                }
                Ok(ConfigValue::Int(*i))
            }
            (ConfigType::Int { min, max }, ConfigValue::Float(f)) if f.fract() == 0.0 => {
                let i = *f as i64;
                if i < *min || i > *max {
                    return Err(format!("out of range [{min}, {max}]: {i}"));
                }
                Ok(ConfigValue::Int(i))
            }
            (ConfigType::Float { min, max }, ConfigValue::Float(f)) => {
                if !f.is_finite() || f < min || f > max {
                    return Err(format!("out of range [{min}, {max}]: {f}"));
                }
                Ok(ConfigValue::Float(*f))
            }
            (ConfigType::Float { min, max }, ConfigValue::Int(i)) => {
                let f = *i as f64;
                if f < *min || f > *max {
                    return Err(format!("out of range [{min}, {max}]: {f}"));
                }
                Ok(ConfigValue::Float(f))
            }
            (ConfigType::Text { max_len }, ConfigValue::Str(s)) => {
                if *max_len > 0 && s.chars().count() > *max_len {
                    return Err(format!("text longer than {max_len} chars"));
                }
                Ok(ConfigValue::Str(s.clone()))
            }
            (ConfigType::Enum { options }, ConfigValue::Str(s)) => {
                if !options.iter().any(|o| o == s) {
                    return Err(format!("not one of {options:?}: {s}"));
                }
                Ok(ConfigValue::Str(s.clone()))
            }
            (ConfigType::StrList, ConfigValue::StrList(v)) => Ok(ConfigValue::StrList(v.clone())),
            (ConfigType::Path, ConfigValue::Str(s)) => {
                if s.trim().is_empty() {
                    return Err("path must not be empty".to_string());
                }
                Ok(ConfigValue::Str(s.clone()))
            }
            (ty, v) => Err(format!("type mismatch: expected {ty:?}, got {v:?}")),
        }
    }

    /// 类型的 JSON-ish 简称，用于日志与前端。
    pub fn kind_name(&self) -> &'static str {
        match self {
            ConfigType::Bool => "bool",
            ConfigType::Int { .. } => "int",
            ConfigType::Float { .. } => "float",
            ConfigType::Text { .. } => "text",
            ConfigType::Enum { .. } => "enum",
            ConfigType::StrList => "str_list",
            ConfigType::Path => "path",
        }
    }
}

/// 配置取值。刻意不区分 `Path`/`Enum` 的存储形态（统一为 `Str`），类型语义由 [`ConfigType`] 承担，
/// 这样 TOML 往返不需要额外包装。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    StrList(Vec<String>),
}

impl ConfigValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConfigValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConfigValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConfigValue::Float(f) => Some(*f),
            ConfigValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConfigValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_str_list(&self) -> Option<&[String]> {
        match self {
            ConfigValue::StrList(v) => Some(v),
            _ => None,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            ConfigValue::Bool(b) => serde_json::Value::Bool(*b),
            ConfigValue::Int(i) => serde_json::Value::Number((*i).into()),
            ConfigValue::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            ConfigValue::Str(s) => serde_json::Value::String(s.clone()),
            ConfigValue::StrList(v) => {
                serde_json::Value::Array(v.iter().map(|s| serde_json::Value::String(s.clone())).collect())
            }
        }
    }
}

/// 单个配置项的 schema 描述。`owner` 为注册它的模块 id（§5.1 由 guard 装配时注册）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigField {
    /// 点分键路径，如 `guard.send_key`（对应 TOML `[guard] send_key`）。
    pub key: String,
    pub ty: ConfigType,
    pub default: ConfigValue,
    /// 界面标签（中文）。
    pub label: String,
    /// 帮助文本，可为空；文案精简（FR-UI-05）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    /// 设置页分类，如 `general` / `guard` / `privacy`（FR-UI-03）。
    pub group: String,
    pub owner: String,
}

impl ConfigField {
    pub fn new(
        key: impl Into<String>,
        ty: ConfigType,
        default: ConfigValue,
        label: impl Into<String>,
        group: impl Into<String>,
        owner: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            ty,
            default,
            label: label.into(),
            help: String::new(),
            group: group.into(),
            owner: owner.into(),
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }
}

/// 本轮流水线可见的只读配置快照（§4 `PipelineContext.config`）。
#[derive(Debug, Clone, Default)]
pub struct ConfigSnapshot {
    values: BTreeMap<String, ConfigValue>,
    version: u64,
}

impl ConfigSnapshot {
    pub fn new(values: BTreeMap<String, ConfigValue>, version: u64) -> Self {
        Self { values, version }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.values.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// 类型不符或键缺失时回退到 `fallback`（调用方给常量默认），保证运行期永不 panic。
    pub fn bool_or(&self, key: &str, fallback: bool) -> bool {
        self.get(key)
            .and_then(ConfigValue::as_bool)
            .unwrap_or(fallback)
    }

    pub fn i64_or(&self, key: &str, fallback: i64) -> i64 {
        self.get(key)
            .and_then(ConfigValue::as_i64)
            .unwrap_or(fallback)
    }

    pub fn u64_or(&self, key: &str, fallback: u64) -> u64 {
        self.i64_or(key, fallback as i64).max(0) as u64
    }

    pub fn f64_or(&self, key: &str, fallback: f64) -> f64 {
        self.get(key)
            .and_then(ConfigValue::as_f64)
            .unwrap_or(fallback)
    }

    pub fn str_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key)
            .and_then(ConfigValue::as_str)
            .unwrap_or(fallback)
    }

    pub fn str_list_or(&self, key: &str, fallback: &[String]) -> Vec<String> {
        self.get(key)
            .and_then(ConfigValue::as_str_list)
            .map(<[String]>::to_vec)
            .unwrap_or_else(|| fallback.to_vec())
    }
}

/// 配置变更广播载荷。
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigChanged {
    pub key: String,
    pub value: ConfigValue,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unknown config key: {0}")]
    UnknownKey(String),
    #[error("invalid value for `{key}`: {reason}")]
    InvalidValue { key: String, reason: String },
    #[error("config field `{key}` already registered by module `{owner}`")]
    DuplicateKey { key: String, owner: String },
    #[error("config parse error: {0}")]
    Parse(String),
    #[error("config io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 配置损坏时的恢复记录（供日志输出）。
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryRecord {
    pub backup: PathBuf,
    pub reason: String,
}

struct Inner {
    path: PathBuf,
    schema: RwLock<BTreeMap<String, ConfigField>>,
    /// 磁盘上读到的原始值（未经 schema 校验）。
    loaded: RwLock<BTreeMap<String, ConfigValue>>,
    /// 生效值（schema 校验后的最终结果）。
    values: RwLock<BTreeMap<String, ConfigValue>>,
    snapshot: RwLock<Arc<ConfigSnapshot>>,
    subscribers: Mutex<Vec<Sender<ConfigChanged>>>,
    version: AtomicU64,
    recoveries: Mutex<Vec<RecoveryRecord>>,
}

/// 配置中心。多线程共享用 `Arc<ConfigCenter>`。
pub struct ConfigCenter {
    inner: Arc<Inner>,
}

impl ConfigCenter {
    /// 打开（或创建）`path` 指向的配置文件。文件缺失 → 空配置；文件损坏 → 备份 + 默认值重建。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        let (loaded, recoveries) = load_or_recover(&path)?;
        let snap = Arc::new(ConfigSnapshot::default());
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                schema: RwLock::new(BTreeMap::new()),
                loaded: RwLock::new(loaded),
                values: RwLock::new(BTreeMap::new()),
                snapshot: RwLock::new(snap),
                subscribers: Mutex::new(Vec::new()),
                version: AtomicU64::new(1),
                recoveries: Mutex::new(recoveries),
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// 本次打开时的配置恢复记录（空表示配置健康）。
    pub fn recoveries(&self) -> Vec<RecoveryRecord> {
        self.inner
            .recoveries
            .lock()
            .expect("recoveries poisoned")
            .clone()
    }

    /// 模块注册自己的配置项。同一模块重复注册同键 → 覆盖（dev 期热重载友好）；
    /// 不同模块抢注同键 → panic（§5.1「init 期 panic：开发期错误，不得进发布」）。
    pub fn register_module(
        &self,
        module_id: &str,
        fields: Vec<ConfigField>,
    ) -> Result<(), ConfigError> {
        let mut schema = self.inner.schema.write().expect("schema poisoned");
        for mut field in fields {
            if let Some(existing) = schema.get(&field.key) {
                assert_eq!(
                    existing.owner, module_id,
                    "配置键 `{}` 已被模块 `{}` 注册，模块 `{}` 不得抢注（§5.1）",
                    field.key, existing.owner, module_id
                );
            }
            // 注册方是 owner 的权威来源：模块只能为自己声明配置项
            field.owner = module_id.to_string();
            schema.insert(field.key.clone(), field);
        }
        Ok(())
    }

    /// 装配收尾：把「schema 默认值 + 磁盘覆盖值」合成生效值，重建快照并规范化落盘。
    ///
    /// 磁盘上不属于任何 schema 的键会被**原样保留**（模块临时禁用时配置不丢）。
    pub fn finalize(&self) -> Result<Arc<ConfigSnapshot>, ConfigError> {
        let schema = self.inner.schema.read().expect("schema poisoned").clone();
        let loaded = self.inner.loaded.read().expect("loaded poisoned").clone();

        let mut values: BTreeMap<String, ConfigValue> = BTreeMap::new();
        for (key, field) in &schema {
            match loaded.get(key) {
                Some(raw) => match field.ty.normalize(raw) {
                    Ok(v) => {
                        values.insert(key.clone(), v);
                    }
                    Err(reason) => {
                        tracing::warn!(
                            key = %key,
                            reason = %reason,
                            "配置项非法，回退默认值（UT-CORE-02 拒绝路径）"
                        );
                        values.insert(key.clone(), field.default.clone());
                    }
                },
                None => {
                    values.insert(key.clone(), field.default.clone());
                }
            }
        }
        for (key, value) in &loaded {
            if !schema.contains_key(key) {
                values.insert(key.clone(), value.clone());
            }
        }

        let version = self.inner.version.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Arc::new(ConfigSnapshot::new(values.clone(), version));

        *self.inner.values.write().expect("values poisoned") = values;
        *self.inner.snapshot.write().expect("snapshot poisoned") = Arc::clone(&snapshot);
        self.save()?;
        Ok(snapshot)
    }

    /// 读取生效值。
    pub fn get(&self, key: &str) -> Option<ConfigValue> {
        self.inner
            .values
            .read()
            .expect("values poisoned")
            .get(key)
            .cloned()
    }

    /// 当前快照（`Arc` 拷贝，运行期零锁竞争）。
    pub fn snapshot(&self) -> Arc<ConfigSnapshot> {
        Arc::clone(&self.inner.snapshot.read().expect("snapshot poisoned"))
    }

    /// 全部配置项 schema（按 key 排序，前端渲染顺序稳定）。
    pub fn schema(&self) -> Vec<ConfigField> {
        self.inner
            .schema
            .read()
            .expect("schema poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 写入配置：先校验，再更新内存与快照，最后原子落盘并广播；任一步失败都不产生副作用。
    pub fn set(&self, key: &str, value: ConfigValue) -> Result<ConfigValue, ConfigError> {
        let field = self
            .inner
            .schema
            .read()
            .expect("schema poisoned")
            .get(key)
            .cloned()
            .ok_or_else(|| ConfigError::UnknownKey(key.to_string()))?;

        let normalized =
            field
                .ty
                .normalize(&value)
                .map_err(|reason| ConfigError::InvalidValue {
                    key: key.to_string(),
                    reason,
                })?;

        // 写锁内完成「更新内存 → 重建快照 → 落盘」，保证并发写不会把旧快照写回文件。
        let mut values = self.inner.values.write().expect("values poisoned");
        values.insert(key.to_string(), normalized.clone());
        let version = self.inner.version.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Arc::new(ConfigSnapshot::new(values.clone(), version));
        *self.inner.snapshot.write().expect("snapshot poisoned") = snapshot;
        self.save_locked(&values)?;
        drop(values);

        let changed = ConfigChanged {
            key: key.to_string(),
            value: normalized.clone(),
        };
        self.broadcast(&changed);
        Ok(normalized)
    }

    /// 写入非 schema 管理的键（主程序壳的内部状态，如窗口位置记忆）。
    ///
    /// 与 [`ConfigCenter::set`] 的分工：`set` 走 schema 校验并广播（模块配置）；
    /// 本方法仅供壳层持久化自有状态——不进 schema（前端设置页不渲染）、
    /// 不广播（无模块订阅这些键）。schema 已注册的键一律拒绝，防止绕过校验。
    /// 一次落盘写多个键（窗口 x/y/width/height 四键同源，逐键写会落盘出中间态）。
    pub fn set_unmanaged_batch(
        &self,
        entries: Vec<(String, ConfigValue)>,
    ) -> Result<(), ConfigError> {
        {
            let schema = self.inner.schema.read().expect("schema poisoned");
            for (key, _) in &entries {
                if schema.contains_key(key) {
                    return Err(ConfigError::InvalidValue {
                        key: key.to_string(),
                        reason: "schema 管理的键必须走 set()，不得绕过校验".into(),
                    });
                }
            }
        }
        let mut values = self.inner.values.write().expect("values poisoned");
        for (key, value) in entries {
            values.insert(key, value);
        }
        let version = self.inner.version.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Arc::new(ConfigSnapshot::new(values.clone(), version));
        *self.inner.snapshot.write().expect("snapshot poisoned") = snapshot;
        self.save_locked(&values)
    }

    /// 订阅配置变更；返回的接收端被丢弃后自动从广播表移除。
    pub fn subscribe(&self) -> Receiver<ConfigChanged> {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.inner
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .push(tx);
        rx
    }

    fn broadcast(&self, changed: &ConfigChanged) {
        let mut subs = self.inner.subscribers.lock().expect("subscribers poisoned");
        subs.retain(|tx| tx.send(changed.clone()).is_ok());
    }

    fn save(&self) -> Result<(), ConfigError> {
        let values = self.inner.values.read().expect("values poisoned");
        self.save_locked(&values)
    }

    fn save_locked(&self, values: &BTreeMap<String, ConfigValue>) -> Result<(), ConfigError> {
        let mut obj = serde_json::Map::new();
        for (key, value) in values {
            insert_json_path(&mut obj, key, value.to_json());
        }
        let text = serde_json::to_string_pretty(&obj).map_err(|e| ConfigError::Parse(e.to_string()))?;
        json_file::write_atomic(&self.inner.path, text.as_bytes())
    }

    /// 从磁盘重新加载配置（热重载）。
    pub fn reload(&self) -> Result<(), ConfigError> {
        let (loaded, recoveries) = load_or_recover(&self.inner.path)?;

        // 追加恢复记录
        if !recoveries.is_empty() {
            let mut rec_guard = self.inner.recoveries.lock().expect("recoveries poisoned");
            rec_guard.extend(recoveries);
        }

        // 合并 schema 默认值 + 已加载值
        let schema = self.inner.schema.read().expect("schema poisoned");
        let mut merged = BTreeMap::new();
        for field in schema.values() {
            let key = &field.key;
            merged.insert(
                key.clone(),
                loaded.get(key).cloned().unwrap_or_else(|| field.default.clone()),
            );
        }

        // 替换运行时值
        let mut values = self.inner.values.write().expect("values poisoned");
        *values = merged;

        Ok(())
    }
}

impl config_doc::Document for ConfigCenter {
    fn name(&self) -> &str {
        "config"
    }

    fn path(&self) -> &Path {
        &self.inner.path
    }

    fn flush(&self) -> Result<(), ConfigError> {
        self.save()
    }

    fn reload(&self) -> Result<(), ConfigError> {
        ConfigCenter::reload(self)
    }

    fn recoveries(&self) -> Vec<RecoveryRecord> {
        self.inner
            .recoveries
            .lock()
            .expect("recoveries poisoned")
            .clone()
    }
}

/// 读取配置；损坏时备份并以空配置继续。
fn load_or_recover(
    path: &Path,
) -> Result<(BTreeMap<String, ConfigValue>, Vec<RecoveryRecord>), ConfigError> {
    if !path.exists() {
        return Ok((BTreeMap::new(), Vec::new()));
    }
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut out = BTreeMap::new();
    let parse_result = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| ConfigError::Parse(e.to_string()))
        .and_then(|v| flatten_json("", &v, &mut out));

    match parse_result {
        Ok(()) => Ok((out, Vec::new())),
        Err(err) => {
            let recovery = json_file::backup_broken(path, &err.to_string())?;
            Ok((BTreeMap::new(), vec![recovery]))
        }
    }
}

fn flatten_json(
    prefix: &str,
    value: &serde_json::Value,
    out: &mut BTreeMap<String, ConfigValue>,
) -> Result<(), ConfigError> {
    match value {
        serde_json::Value::Object(obj) => {
            for (k, v) in obj {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_json(&key, v, out)?;
            }
            Ok(())
        }
        serde_json::Value::Bool(b) => {
            out.insert(prefix.to_string(), ConfigValue::Bool(*b));
            Ok(())
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.insert(prefix.to_string(), ConfigValue::Int(i));
            } else if let Some(f) = n.as_f64() {
                out.insert(prefix.to_string(), ConfigValue::Float(f));
            } else {
                return Err(ConfigError::Parse(format!(
                    "`{prefix}` 数字取值超出范围: {n}"
                )));
            }
            Ok(())
        }
        serde_json::Value::String(s) => {
            out.insert(prefix.to_string(), ConfigValue::Str(s.clone()));
            Ok(())
        }
        serde_json::Value::Array(items) => {
            let mut list = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    serde_json::Value::String(s) => list.push(s.clone()),
                    other => {
                        return Err(ConfigError::Parse(format!(
                            "`{prefix}` 数组含非字符串元素: {other:?}"
                        )))
                    }
                }
            }
            out.insert(prefix.to_string(), ConfigValue::StrList(list));
            Ok(())
        }
        serde_json::Value::Null => Err(ConfigError::Parse(format!(
            "`{prefix}` 取值为 null"
        ))),
    }
}

fn insert_json_path(obj: &mut serde_json::Map<String, serde_json::Value>, key: &str, value: serde_json::Value) {
    match key.split_once('.') {
        None => {
            obj.insert(key.to_string(), value);
        }
        Some((head, rest)) => {
            let entry = obj
                .entry(head.to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if !entry.is_object() {
                *entry = serde_json::Value::Object(serde_json::Map::new());
            }
            if let Some(inner) = entry.as_object_mut() {
                insert_json_path(inner, rest, value);
            }
        }
    }
}
