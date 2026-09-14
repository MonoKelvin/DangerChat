//! 11 条 commands（§5.9 表）。`State<AppState>` 经 `app.manage` 注入。

use tauri::State;

use dc_core::{ConfigType, ConfigValue};
use dc_pipeline::intercept::{AlertAction, GuardState};

use crate::dto::StatusPayload;
use crate::state::AppState;

#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum BridgeError {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Model(String),
    #[error("弹窗动作失败：{0}")]
    Alert(String),
}

pub type CmdResult<T> = Result<T, BridgeError>;

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// 配置项的序列化形态（schema 驱动 UI 渲染，FR-SYS-01）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigFieldDto {
    pub key: String,
    /// bool | int | float | text | enum | strlist | path（ConfigType 的字符串化）
    pub ty: String,
    pub default: serde_json::Value,
    pub label: String,
    pub help: String,
    pub group: String,
}

fn ty_str(t: &ConfigType) -> &'static str {
    match t {
        ConfigType::Bool => "bool",
        ConfigType::Int { .. } => "int",
        ConfigType::Float { .. } => "float",
        ConfigType::Text { .. } => "text",
        ConfigType::Enum { .. } => "enum",
        ConfigType::StrList => "strlist",
        ConfigType::Path => "path",
    }
}

fn value_json(v: &ConfigValue) -> serde_json::Value {
    match v {
        ConfigValue::Bool(b) => serde_json::Value::Bool(*b),
        ConfigValue::Int(i) => (*i).into(),
        ConfigValue::Float(f) => (*f).into(),
        ConfigValue::Str(s) => s.clone().into(),
        ConfigValue::StrList(l) => l.clone().into(),
    }
}

#[tauri::command]
pub fn get_config_schema(state: State<'_, AppState>) -> CmdResult<Vec<ConfigFieldDto>> {
    Ok(state
        .config
        .schema()
        .into_iter()
        .map(|f| ConfigFieldDto {
            key: f.key,
            ty: ty_str(&f.ty).to_string(),
            default: value_json(&f.default),
            label: f.label,
            help: f.help,
            group: f.group,
        })
        .collect())
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>, path: String) -> CmdResult<serde_json::Value> {
    let snap = state.config.snapshot();
    match snap.get(&path) {
        Some(v) => Ok(value_json(v)),
        None => Err(BridgeError::Config(format!("配置项不存在：{path}"))),
    }
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, path: String, value: serde_json::Value) -> CmdResult<serde_json::Value> {
    let cv: ConfigValue = serde_json::from_value(value)
        .map_err(|e| BridgeError::Config(format!("值类型非法：{e}")))?;
    let applied = state
        .config
        .set(&path, cv)
        .map_err(|e| BridgeError::Config(e.to_string()))?;
    // 目标进程名变化 → 通知 window_watch 重新发现（下轮 tick 自愈，无需即时打断）
    if path == "target.process_name" {
        if let ConfigValue::Str(name) = &applied {
            state.set_target_process(name);
        }
    }
    Ok(value_json(&applied))
}

// ---------------------------------------------------------------------------
// 画像 / 违禁词库（直接读写文件，热重载由模块下轮 init/快照实现）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_contacts(state: State<'_, AppState>) -> CmdResult<Vec<ContactDto>> {
    let path = state.data_dir.join("contacts.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let parsed: TomlContacts = toml_parse(&text)?;
    Ok(parsed
        .contact
        .into_iter()
        .map(|c| ContactDto {
            name: c.name,
            profile: c.profile,
        })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContactDto {
    pub name: String,
    pub profile: String,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TomlContacts {
    #[serde(default)]
    contact: Vec<ContactDef>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ContactDef {
    name: String,
    profile: String,
}

fn toml_parse<T: serde::de::DeserializeOwned>(text: &str) -> CmdResult<T> {
    toml::from_str(text).map_err(|e| BridgeError::Config(format!("toml 解析失败：{e}")))
}

#[tauri::command]
pub fn set_contact_profile(state: State<'_, AppState>, name: String, profile: String) -> CmdResult<()> {
    if !matches!(profile.as_str(), "formal" | "casual" | "none") {
        return Err(BridgeError::Config(format!("画像非法：{profile}（formal|casual|none）")));
    }
    let path = state.data_dir.join("contacts.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut parsed: TomlContacts = toml_parse(&text)?;
    // none = 删除该条目（回落默认 formal）
    parsed.contact.retain(|c| c.name != name);
    if profile != "none" {
        parsed.contact.push(ContactDef {
            name: name.clone(),
            profile: profile.clone(),
        });
    }
    let out = toml::to_string_pretty(&parsed)
        .map_err(|e| BridgeError::Config(format!("toml 序列化失败：{e}")))?;
    std::fs::write(&path, out).map_err(|e| BridgeError::Io(e.to_string()))?;
    tracing::info!(contact = %name, profile = %profile, "画像已更新");
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuleDto {
    pub pattern: String,
    /// word | substring | regex
    pub r#match: String,
    /// warn | block
    pub severity: String,
    /// formal | casual | all 列表
    pub applies_to: Vec<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TomlRules {
    #[serde(default)]
    rule: Vec<RuleDto>,
}

#[tauri::command]
pub fn get_rules(state: State<'_, AppState>) -> CmdResult<Vec<RuleDto>> {
    let path = state.data_dir.join("rules.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Ok(toml_parse::<TomlRules>(&text)?.rule)
}

#[tauri::command]
pub fn save_rules(state: State<'_, AppState>, rules: Vec<RuleDto>) -> CmdResult<()> {
    // 先校验全部可解析（整批原子：任一非法拒绝保存）
    let ruleset = dc_pipeline::sem::rules::RuleSet::from_defs(
        rules
            .iter()
            .map(|r| dc_pipeline::sem::rules::RuleDef {
                pattern: r.pattern.clone(),
                r#match: serde_json::from_value(serde_json::Value::String(r.r#match.clone()))
                    .unwrap_or(dc_pipeline::sem::rules::MatchKind::Substring),
                severity: serde_json::from_value(serde_json::Value::String(r.severity.clone()))
                    .unwrap_or(dc_pipeline::sem::rules::Severity::Warn),
                applies_to: r.applies_to.clone(),
            })
            .collect(),
    )
    .map_err(|e| BridgeError::Config(format!("规则校验失败：{e}")))?;
    let _ = ruleset;
    let out = toml::to_string_pretty(&TomlRules { rule: rules.clone() })
        .map_err(|e| BridgeError::Config(format!("toml 序列化失败：{e}")))?;
    let path = state.data_dir.join("rules.toml");
    std::fs::write(&path, out).map_err(|e| BridgeError::Io(e.to_string()))?;
    tracing::info!(count = rules.len(), "规则库已保存");
    Ok(())
}

// ---------------------------------------------------------------------------
// 模型 / 守护控制 / 日志 / 弹窗
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn import_model(state: State<'_, AppState>, kind: String, path: String) -> CmdResult<()> {
    let k = dc_core::ModelKind::parse(&kind)
        .ok_or_else(|| BridgeError::Model(format!("模型类别非法：{kind}（layout|sem|ocr）")))?;
    let required: Vec<String> = Vec::new();
    state
        .models
        .import(k, std::path::Path::new(&path), &required)
        .map_err(|e| BridgeError::Model(e.to_string()))?;
    tracing::info!(kind = %kind, path = %path, "模型已导入");
    Ok(())
}

#[tauri::command]
pub fn pause_guard(state: State<'_, AppState>) -> CmdResult<()> {
    state.intercept.set_paused(true);
    Ok(())
}

#[tauri::command]
pub fn resume_guard(state: State<'_, AppState>) -> CmdResult<()> {
    state.intercept.set_paused(false);
    Ok(())
}

#[tauri::command]
pub fn get_guard_status(state: State<'_, AppState>) -> CmdResult<StatusPayload> {
    Ok(status_of(&state))
}

/// 状态载荷组装（pump 的状态迁移事件同用）。
pub fn status_of(state: &AppState) -> StatusPayload {
    let s = state.intercept.state();
    StatusPayload {
        state: guard_state_str(s).to_string(),
        target_process: state.target_process(),
        target_found: state.guard_running(),
    }
}

fn guard_state_str(s: GuardState) -> &'static str {
    match s {
        GuardState::Active => "active",
        GuardState::Suspended => "suspended",
        GuardState::Paused => "paused",
        GuardState::Cooldown => "cooldown",
    }
}

#[tauri::command]
pub fn clear_logs(state: State<'_, AppState>) -> CmdResult<()> {
    // LogCenter::clear_all 是实例方法；bootstrap 持有的实例经 AppState 暴露
    if let Some(lc) = &state.log_center {
        lc.clear_all().map_err(|e| BridgeError::Io(e.to_string()))?;
    }
    tracing::info!("日志已清空（用户主动）");
    Ok(())
}

/// 弹窗回执（FR-BRG-03：allow → 内部 allow-once 标志，**无任何按键注入**）。
#[tauri::command]
pub fn alert_action(state: State<'_, AppState>, action: String) -> CmdResult<()> {
    let a = match action.as_str() {
        "allow" => AlertAction::Allow,
        "cancel" => AlertAction::Cancel,
        "edit" => AlertAction::Edit,
        "snooze" => AlertAction::Snooze,
        other => return Err(BridgeError::Alert(format!("未知动作：{other}"))),
    };
    // apply_alert_action 无返回值（内部走 Intercept 的原子状态机）
    state.intercept.apply_alert_action(a);
    Ok(())
}
