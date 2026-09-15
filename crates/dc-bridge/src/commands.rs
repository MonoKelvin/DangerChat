//! 11 条 commands（§5.9 表）。`State<AppState>` 经 `app.manage` 注入。

use std::sync::Arc;

use tauri::{Manager, State};

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
    /// enum 类型的可选值（其他类型为空）
    #[serde(default)]
    pub options: Vec<String>,
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

fn type_options(t: &ConfigType) -> Vec<String> {
    match t {
        ConfigType::Enum { options } => options.clone(),
        _ => Vec::new(),
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
pub fn get_config_schema(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<ConfigFieldDto>> {
    Ok(state
        .config
        .schema()
        .into_iter()
        .map(|f| ConfigFieldDto {
            key: f.key,
            ty: ty_str(&f.ty).to_string(),
            default: value_json(&f.default),
            options: type_options(&f.ty),
            label: f.label,
            help: f.help,
            group: f.group,
        })
        .collect())
}

#[tauri::command]
pub fn get_config(state: State<'_, Arc<AppState>>, path: String) -> CmdResult<serde_json::Value> {
    let snap = state.config.snapshot();
    match snap.get(&path) {
        Some(v) => Ok(value_json(v)),
        None => Err(BridgeError::Config(format!("配置项不存在：{path}"))),
    }
}

#[tauri::command]
pub fn set_config(state: State<'_, Arc<AppState>>, path: String, value: serde_json::Value) -> CmdResult<serde_json::Value> {
    let cv: ConfigValue = serde_json::from_value(value)
        .map_err(|e| BridgeError::Config(format!("值类型非法：{e}")))?;
    let applied = state
        .config
        .set(&path, cv)
        .map_err(|e| BridgeError::Config(e.to_string()))?;
    // 运行时热更新：目标进程名 / 发送键 / 守护开关改完即时生效（无需重启）
    match path.as_str() {
        "target.process_name" => {
            if let ConfigValue::Str(name) = &applied {
                state.set_target_process(name);
                state.intercept.set_target_process(name);
            }
        }
        "guard.send_key" => {
            if let ConfigValue::Str(v) = &applied {
                state
                    .intercept
                    .set_send_key(dc_pipeline::intercept::SendKey::parse(v));
            }
        }
        "guard.enabled" => {
            if let ConfigValue::Bool(on) = &applied {
                state.intercept.set_enabled(*on);
            }
        }
        "guard.verdict_ttl_ms" => {
            if let ConfigValue::Int(ms) = &applied {
                state.intercept.set_verdict_ttl_ms(*ms as u64);
            }
        }
        _ => {}
    }
    Ok(value_json(&applied))
}

// ---------------------------------------------------------------------------
// 画像 / 违禁词库（直接读写文件，热重载由模块下轮 init/快照实现）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_contacts(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<ContactDto>> {
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
pub fn set_contact_profile(state: State<'_, Arc<AppState>>, name: String, profile: String) -> CmdResult<()> {
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
pub fn get_rules(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<RuleDto>> {
    let path = state.data_dir.join("rules.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Ok(toml_parse::<TomlRules>(&text)?.rule)
}

#[tauri::command]
pub fn save_rules(state: State<'_, Arc<AppState>>, rules: Vec<RuleDto>) -> CmdResult<()> {
    // 空 pattern 行是编辑器草稿，不参与校验也不落盘（重启后自然消失）
    let rules: Vec<RuleDto> = rules
        .into_iter()
        .filter(|r| !r.pattern.trim().is_empty())
        .collect();

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
    tracing::info!(count = rules.len(), path = %path.display(), "规则库已保存");

    // Guard 运行中 → 即时热重载（未运行则下次发现目标窗口时 init 读到新文件）
    if let Ok(guard_opt) = state.guard.lock() {
        if let Some(guard) = guard_opt.as_ref() {
            let rules_path = path.to_string_lossy().to_string();
            let contacts_path = state.data_dir.join("contacts.toml").to_string_lossy().to_string();
            guard.reload_rules(&rules_path, &contacts_path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 模型 / 守护控制 / 日志 / 弹窗
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn import_model(state: State<'_, Arc<AppState>>, kind: String, path: String) -> CmdResult<()> {
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
pub fn pause_guard(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    state.intercept.set_paused(true);
    Ok(())
}

#[tauri::command]
pub fn resume_guard(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    state.intercept.set_paused(false);
    Ok(())
}

#[tauri::command]
pub fn get_guard_status(state: State<'_, Arc<AppState>>) -> CmdResult<StatusPayload> {
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
pub fn clear_logs(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    // LogCenter::clear_all 是实例方法；bootstrap 持有的实例经 AppState 暴露
    if let Some(lc) = &state.log_center {
        lc.clear_all().map_err(|e| BridgeError::Io(e.to_string()))?;
    }
    tracing::info!("日志已清空（用户主动）");
    Ok(())
}

// ---------------------------------------------------------------------------
// 数据目录（FR：用户可将全部数据迁移到自选位置）
// ---------------------------------------------------------------------------

/// 前端主题色切换 → 托盘图标同步按该色相着色（不生成多张图）。
/// 主壳命令负责调用 + 刷新托盘（那边有 AppHandle）。
pub fn set_tray_hue_inner(state: &Arc<AppState>, hue: u32) {
    state.tray_hue_deg.store(hue.min(359), std::sync::atomic::Ordering::Relaxed);
}

#[derive(Debug, serde::Serialize)]
pub struct DataDirInfo {
    /// 当前生效目录
    pub current: String,
    /// 是否为用户自定义（非系统默认）
    pub custom: bool,
}

#[tauri::command]
pub fn get_data_dir(state: State<'_, Arc<AppState>>) -> CmdResult<DataDirInfo> {
    let current = state.data_dir.display().to_string();
    let same = state
        .data_dir
        .canonicalize()
        .ok()
        .and_then(|c| state.default_data_dir.canonicalize().ok().map(|d| c == d))
        .unwrap_or(state.data_dir == state.default_data_dir);
    Ok(DataDirInfo {
        current,
        custom: !same,
    })
}

/// 弹文件夹选择框（rfd）。用户取消 → Ok(None)。
/// async 命令跑在 Tauri 工作线程（非主线程），同步弹窗不冻结 UI。
#[tauri::command]
pub async fn pick_data_dir() -> CmdResult<Option<String>> {
    let picked = rfd::FileDialog::new()
        .set_title("选择数据保存目录")
        .pick_folder()
        .map(|p| p.display().to_string());
    Ok(picked)
}

/// 写入自定义数据目录指针（重启后生效）。空字符串 = 恢复系统默认。
#[tauri::command]
pub fn set_data_dir(state: State<'_, Arc<AppState>>, path: String) -> CmdResult<()> {
    let pointer = crate::state::data_dir_pointer(&state.default_data_dir);
    if path.trim().is_empty() {
        std::fs::remove_file(&pointer).ok();
        tracing::info!("数据目录已恢复系统默认（重启后生效）");
        return Ok(());
    }
    let dir = std::path::PathBuf::from(path.trim());
    std::fs::create_dir_all(&dir).map_err(|e| BridgeError::Io(format!("目录创建失败：{e}")))?;
    std::fs::write(&pointer, path.trim())
        .map_err(|e| BridgeError::Io(format!("指针写入失败：{e}")))?;
    tracing::info!(dir = %dir.display(), "数据目录已设置（重启后生效）");
    Ok(())
}

/// 资源管理器打开当前数据目录。
#[tauri::command]
pub fn open_data_dir(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    std::process::Command::new("explorer")
        .arg(&state.data_dir)
        .spawn()
        .map_err(|e| BridgeError::Io(e.to_string()))?;
    Ok(())
}

/// 弹窗回执（FR-BRG-03：allow → 内部 allow-once 标志，**无任何按键注入**）。
/// 后端直接隐藏 alert 窗口（兜底：前端 hide 曾因 capabilities 缺权限被静默拒绝）。
#[tauri::command]
pub fn alert_action(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    action: String,
) -> CmdResult<()> {
    let a = match action.as_str() {
        "allow" => AlertAction::Allow,
        "cancel" => AlertAction::Cancel,
        "edit" => AlertAction::Edit,
        "snooze" => AlertAction::Snooze,
        other => return Err(BridgeError::Alert(format!("未知动作：{other}"))),
    };
    // apply_alert_action 无返回值（内部走 Intercept 的原子状态机）
    state.intercept.apply_alert_action(a);
    if let Some(win) = app.get_webview_window("alert") {
        let _ = win.hide();
    }
    Ok(())
}
