//! 11 条 commands（§5.9 表）。`State<AppState>` 经 `app.manage` 注入。

use std::sync::Arc;

use tauri::{Manager, State};

use dc_core::{ConfigType, ConfigValue};
use dc_pipeline::intercept::{AlertAction, GuardState};

use crate::dto::{DatasetDto, ModelDto, StatusPayload, TrainingStatusDto};
use crate::events;
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
pub fn set_config(
    state: State<'_, Arc<AppState>>,
    path: String,
    value: serde_json::Value,
) -> CmdResult<serde_json::Value> {
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
// 画像 / 违禁词库 / 场景（直接读写文件，热重载由模块下轮 init/快照实现）
// ---------------------------------------------------------------------------

/// Guard 运行中 → 即时热重载 sem 的三份数据（规则/画像/场景）。
/// 未运行则下次发现目标窗口时 init 读到新文件。
fn trigger_sem_reload(state: &AppState) {
    if let Ok(guard_opt) = state.guard.lock() {
        if let Some(guard) = guard_opt.as_ref() {
            let rules_path = state
                .data_dir
                .join("rules.toml")
                .to_string_lossy()
                .to_string();
            let contacts_path = state
                .data_dir
                .join("contacts.toml")
                .to_string_lossy()
                .to_string();
            let scenes_path = state
                .data_dir
                .join("scenes.toml")
                .to_string_lossy()
                .to_string();
            guard.reload_rules(&rules_path, &contacts_path, &scenes_path);
        }
    }
}

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
pub fn set_contact_profile(
    state: State<'_, Arc<AppState>>,
    name: String,
    profile: String,
) -> CmdResult<()> {
    // none = 删除条目（回落默认 formal）；其余必须是已存在的场景 id
    let known = state
        .scenarios
        .lock()
        .map(|s| s.is_known(&profile))
        .unwrap_or(false);
    if profile != "none" && !known {
        return Err(BridgeError::Config(format!(
            "画像非法：{profile}（不存在的场景）"
        )));
    }
    let path = state.data_dir.join("contacts.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut parsed: TomlContacts = toml_parse(&text)?;
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
    trigger_sem_reload(&state);
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuleDto {
    pub pattern: String,
    /// word | substring | regex
    pub r#match: String,
    /// 场景 id 列表（或 all）
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
                applies_to: r.applies_to.clone(),
            })
            .collect(),
    )
    .map_err(|e| BridgeError::Config(format!("规则校验失败：{e}")))?;
    let _ = ruleset;
    let out = toml::to_string_pretty(&TomlRules {
        rule: rules.clone(),
    })
    .map_err(|e| BridgeError::Config(format!("toml 序列化失败：{e}")))?;
    let path = state.data_dir.join("rules.toml");
    std::fs::write(&path, out).map_err(|e| BridgeError::Io(e.to_string()))?;
    tracing::info!(count = rules.len(), path = %path.display(), "规则库已保存");

    trigger_sem_reload(&state);
    Ok(())
}

// ---------------------------------------------------------------------------
// 场景管理（ScenarioManager 的命令面；内置「正式」「个人」+ 自定义最多共 10 个）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScenarioDto {
    pub id: String,
    pub name: String,
    /// formal | casual（L2 判定基线）
    pub base: String,
    pub fixed: bool,
}

fn parse_base(base: &str) -> CmdResult<dc_pipeline::sem::rules::Profile> {
    match base {
        "formal" => Ok(dc_pipeline::sem::rules::Profile::Formal),
        "casual" => Ok(dc_pipeline::sem::rules::Profile::Casual),
        other => Err(BridgeError::Config(format!(
            "基线非法：{other}（formal|casual）"
        ))),
    }
}

fn with_scenarios<T>(
    state: &State<'_, Arc<AppState>>,
    f: impl FnOnce(&mut dc_pipeline::sem::scenarios::ScenarioManager) -> CmdResult<T>,
) -> CmdResult<T> {
    let mut guard = state
        .scenarios
        .lock()
        .map_err(|_| BridgeError::Config("场景管理器不可用".into()))?;
    f(&mut guard)
}

#[tauri::command]
pub fn list_scenarios(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<ScenarioDto>> {
    with_scenarios(&state, |m| {
        Ok(m.all()
            .into_iter()
            .map(|s| ScenarioDto {
                id: s.id,
                name: s.name,
                base: s.base.as_str().into(),
                fixed: s.fixed,
            })
            .collect())
    })
}

#[tauri::command]
pub fn add_scenario(
    state: State<'_, Arc<AppState>>,
    name: String,
    base: String,
) -> CmdResult<ScenarioDto> {
    let base = parse_base(&base)?;
    let dto = with_scenarios(&state, |m| {
        m.add(&name, base)
            .map(|s| ScenarioDto {
                id: s.id,
                name: s.name,
                base: s.base.as_str().into(),
                fixed: s.fixed,
            })
            .map_err(BridgeError::Config)
    })?;
    trigger_sem_reload(&state);
    Ok(dto)
}

#[tauri::command]
pub fn update_scenario(
    state: State<'_, Arc<AppState>>,
    id: String,
    name: String,
    base: String,
) -> CmdResult<ScenarioDto> {
    let base = parse_base(&base)?;
    let dto = with_scenarios(&state, |m| {
        m.update(&id, &name, base)
            .map(|s| ScenarioDto {
                id: s.id,
                name: s.name,
                base: s.base.as_str().into(),
                fixed: s.fixed,
            })
            .map_err(BridgeError::Config)
    })?;
    trigger_sem_reload(&state);
    Ok(dto)
}

#[tauri::command]
pub fn remove_scenario(state: State<'_, Arc<AppState>>, id: String) -> CmdResult<()> {
    with_scenarios(&state, |m| m.remove(&id).map_err(BridgeError::Config))?;
    trigger_sem_reload(&state);
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

// ---------------------------------------------------------------------------
// 模型 / 训练数据 / 自助训练（FR-ROI-04）
// ---------------------------------------------------------------------------

/// 扫描 models/ 全部有效模型（dir_watch 与 list_models 共用；非法目录由 ModelStore 跳过）。
pub fn scan_models(state: &AppState) -> Vec<ModelDto> {
    state
        .models
        .list()
        .into_iter()
        .map(|m| ModelDto {
            name: m.name,
            kind: m.meta.kind,
            version: m.meta.version,
            note: m.meta.note,
        })
        .collect()
}

#[tauri::command]
pub fn list_models(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<ModelDto>> {
    Ok(scan_models(&state))
}

/// 扫描数据目录 datasets/ 下的训练数据 zip（按修改时间倒序，最新导出的在前）。
pub fn scan_datasets(state: &AppState) -> Vec<DatasetDto> {
    let dir = state.data_dir.join("datasets");
    let _ = std::fs::create_dir_all(&dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<DatasetDto> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("zip"))
        })
        .filter_map(|e| {
            let size = e.metadata().ok().map(|m| m.len()).unwrap_or(0);
            e.file_name().to_str().map(|n| DatasetDto {
                name: n.to_string(),
                size_bytes: size,
            })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

#[tauri::command]
pub fn list_datasets(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<DatasetDto>> {
    Ok(scan_datasets(&state))
}

#[tauri::command]
pub fn training_status(state: State<'_, Arc<AppState>>) -> CmdResult<TrainingStatusDto> {
    Ok(state
        .training
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone())
}

/// 找 uitag.exe：发布布局（主程序旁）→ 仓库开发布局（apps/uitag 的 target 产物）。
fn find_uitag_exe() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    for cand in [
        exe_dir.join("uitag.exe"),
        exe_dir.join("tools/uitag/uitag.exe"),
    ] {
        if cand.is_file() {
            return Some(cand);
        }
    }
    exe.ancestors().skip(1).find_map(|root| {
        ["release", "debug"].iter().find_map(|profile| {
            let p = root
                .join("apps/uitag/src-tauri/target")
                .join(profile)
                .join("uitag.exe");
            p.is_file().then_some(p)
        })
    })
}

/// 启动 uitag 打标工具（独立进程；找不到可执行文件时给出指引）。
#[tauri::command]
pub fn launch_uitag() -> CmdResult<()> {
    let exe = find_uitag_exe()
        .ok_or_else(|| BridgeError::Io("未找到 uitag.exe（开发布局需先构建 apps/uitag）".into()))?;
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| BridgeError::Io(format!("uitag 启动失败：{e}")))?;
    tracing::info!(exe = %exe.display(), "uitag 已启动");
    Ok(())
}

/// 找训练脚本：发布布局（exe 旁 tools/）→ 仓库开发布局（exe 向上找仓库根）。
fn find_training_script() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let bundled = exe_dir.join("tools/training/train_layout.py");
    if bundled.is_file() {
        return Some(bundled);
    }
    exe.ancestors()
        .skip(1)
        .map(|root| root.join("tools/training/train_layout.py"))
        .find(|p| p.is_file())
}

/// 控制台子进程抑制黑框（python 是控制台程序，GUI 父进程直接 spawn 会闪 cmd 窗口）。
fn no_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// 取输出尾部（训练失败时给用户看 Python 的报错摘要）。
fn output_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(400);
    chars[start..].iter().collect::<String>().trim().to_string()
}

/// 数据集 zip 主干 → 模型目录名（只留安全字符，防路径注入）。
fn model_name_of(dataset: &str) -> String {
    let stem = dataset.trim_end_matches(".zip");
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = format!("layout-{cleaned}");
    name.chars().take(64).collect()
}

/// 后台执行训练：python train_layout.py <zip> --out <models/> --name <layout-*>。
/// 状态经 `training` 字段 + `training://status` 事件同步前端。
fn run_training(app: tauri::AppHandle, state: Arc<AppState>, zip: std::path::PathBuf) {
    let dataset = zip
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let model_name = model_name_of(&dataset);

    let set_status = |s: TrainingStatusDto| {
        if let Ok(mut t) = state.training.lock() {
            *t = s.clone();
        }
        use tauri::Emitter as _;
        let _ = app.emit(events::EVENT_TRAINING, &s);
    };

    let Some(script) = find_training_script() else {
        set_status(TrainingStatusDto {
            state: "error".into(),
            dataset,
            model: None,
            message: Some("未找到训练脚本 tools/training/train_layout.py".into()),
        });
        return;
    };

    let mut last_err = String::new();
    for py in ["python", "py"] {
        let mut cmd = std::process::Command::new(py);
        cmd.arg(&script)
            .arg(&zip)
            .arg("--out")
            .arg(state.models.root())
            .arg("--name")
            .arg(&model_name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        no_console(&mut cmd);
        match cmd.output() {
            Ok(o) if o.status.success() => {
                tracing::info!(model = %model_name, "自助训练完成");
                set_status(TrainingStatusDto {
                    state: "success".into(),
                    dataset,
                    model: Some(model_name),
                    message: None,
                });
                return;
            }
            Ok(o) => {
                // 解释器在但训练失败（缺依赖/数据非法）：带 Python 报错摘要返回
                last_err = format!(
                    "训练失败：{}",
                    if output_tail(&o.stderr).is_empty() {
                        output_tail(&o.stdout)
                    } else {
                        output_tail(&o.stderr)
                    }
                );
                break;
            }
            Err(_) => continue, // 该解释器不存在，试下一个
        }
    }

    let message = if last_err.is_empty() {
        "未找到 Python（需要 python 或 py 在 PATH，且已安装 ultralytics）".to_string()
    } else {
        last_err
    };
    tracing::warn!(model = %model_name, %message, "自助训练失败");
    set_status(TrainingStatusDto {
        state: "error".into(),
        dataset,
        model: None,
        message: Some(message),
    });
}

/// 发起自助训练（立即返回，进度走 `training://status` 事件）。
#[tauri::command]
pub fn start_training(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    dataset: String,
) -> CmdResult<()> {
    // dataset 只允许文件名（拼接 datasets/ 前缀，防 ../ 逃逸）
    if dataset.is_empty()
        || dataset.contains(['/', '\\'])
        || dataset.contains("..")
        || !dataset.to_ascii_lowercase().ends_with(".zip")
    {
        return Err(BridgeError::Model(format!("训练数据名非法：{dataset}")));
    }
    let zip = state.data_dir.join("datasets").join(&dataset);
    if !zip.is_file() {
        return Err(BridgeError::Model(format!("训练数据不存在：{dataset}")));
    }

    // 并发拒绝：同一时间只允许一个训练任务
    {
        let mut t = state.training.lock().unwrap_or_else(|e| e.into_inner());
        if t.state == "running" {
            return Err(BridgeError::Model("已有训练任务进行中".into()));
        }
        *t = TrainingStatusDto {
            state: "running".into(),
            dataset: dataset.clone(),
            model: None,
            message: None,
        };
    }

    let handle = app.app_handle().clone();
    let state = state.inner().clone();
    std::thread::Builder::new()
        .name("layout-training".into())
        .spawn(move || run_training(handle, state, zip))
        .map_err(|e| BridgeError::Io(format!("训练线程启动失败：{e}")))?;
    tracing::info!(%dataset, "自助训练开始");
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
    state
        .tray_hue_deg
        .store(hue.min(359), std::sync::atomic::Ordering::Relaxed);
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

/// 迁移结果（前端据此展示回执 + 决定是否清理旧目录）。
#[derive(Debug, Default, serde::Serialize)]
pub struct MigrateReport {
    /// 已复制的文件数
    pub files: u64,
    /// 已复制的总字节数
    pub bytes: u64,
    /// 目标目录（迁移后生效位置）
    pub target: String,
    /// 迁移前的旧目录（供前端「删除旧目录」选项使用；后端会二次校验再删）
    pub previous: String,
}

/// 递归复制目录内容（不复制子目录本身，只复制其内容）。
/// 返回 (文件数, 总字节数)。已存在的同名文件直接覆盖。
fn copy_dir_contents(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<(u64, u64)> {
    let mut files = 0u64;
    let mut bytes = 0u64;
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = entry.metadata()?;
        if meta.is_dir() {
            let (f, b) = copy_dir_contents(&from, &to)?;
            files += f;
            bytes += b;
        } else if meta.is_file() {
            // 跳过「数据目录指针」自身：它必须留在默认目录，否则自引用
            if entry.file_name() == "data_dir.txt" {
                continue;
            }
            std::fs::copy(&from, &to)?;
            files += 1;
            bytes += meta.len();
        }
        // 符号链接等其它类型跳过（本项目不产出）
    }
    Ok((files, bytes))
}

/// 一键迁移数据目录：把当前目录全部内容复制到 `target`，成功后写入指针（重启生效）。
///
/// 设计取舍（fail-safe）：
/// - **不删除**旧目录内容。迁移失败/用户反悔时可原样回退；旧目录由用户自行清理。
/// - 指针写入是唯一「提交点」——复制阶段任何失败都不会改变生效目录。
/// - Guard 运行中会持有日志/模型文件句柄，复制只读、不冲突；但新目录需重启后生效
///   （与 set_data_dir 一致，避免运行期切换根目录导致句柄失效）。
#[tauri::command]
pub fn migrate_data_dir(
    state: State<'_, Arc<AppState>>,
    target: String,
) -> CmdResult<MigrateReport> {
    let target = target.trim();
    if target.is_empty() {
        return Err(BridgeError::Config("迁移目标目录为空".into()));
    }
    let dst = std::path::PathBuf::from(target);
    let src = state.data_dir.clone();

    // 目标与当前相同（含规范化后相同）→ 无事可做
    let same = |a: &std::path::Path, b: &std::path::Path| -> bool {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => a == b,
        }
    };
    if same(&src, &dst) {
        return Err(BridgeError::Config("目标目录与当前数据目录相同".into()));
    }

    std::fs::create_dir_all(&dst).map_err(|e| BridgeError::Io(format!("目标目录创建失败：{e}")))?;

    // 反向包含检查（置于创建之后：canonicalize 需要目录已存在）。
    // 目标在当前目录内部会把数据递归复制进自身。
    if let (Ok(s), Ok(d)) = (src.canonicalize(), dst.canonicalize()) {
        if d.starts_with(&s) {
            return Err(BridgeError::Config(
                "目标目录位于当前数据目录内部，请另选位置".into(),
            ));
        }
    }

    let (files, bytes) =
        copy_dir_contents(&src, &dst).map_err(|e| BridgeError::Io(format!("数据复制失败：{e}")))?;

    // 提交点：写指针（指向新目录），重启后生效
    let pointer = crate::state::data_dir_pointer(&state.default_data_dir);
    std::fs::write(&pointer, target).map_err(|e| BridgeError::Io(format!("指针写入失败：{e}")))?;

    // 登记待清理旧目录：本进程的 data_dir 仍是旧目录（启动时解析、运行期不变），
    // 只有经此登记，「删除旧目录」才会被放行（见 delete_old_data_dir）。
    if let Ok(mut slot) = state.pending_cleanup.lock() {
        *slot = Some(src.clone());
    }

    tracing::info!(
        from = %src.display(),
        to = %dst.display(),
        files,
        bytes,
        "数据目录已迁移（旧目录保留，重启后生效）"
    );
    Ok(MigrateReport {
        files,
        bytes,
        target: dst.display().to_string(),
        previous: src.display().to_string(),
    })
}

/// 删除目标校验（纯函数，便于单测）：返回 Ok(规范化路径) 或拒绝原因。
///
/// 采用**白名单**语义：只有 `pending_cleanup` 中登记的目录（即刚刚迁移走的旧目录）
/// 才允许删除。这比「排除当前/默认目录」的黑名单更安全 —— 后者在迁移场景下
/// 会与 `data_dir` 启动时解析、运行期不变的特性冲突，导致旧目录永远删不掉。
///
/// 另叠加独立红线（即使路径已登记也不放行）：磁盘根、用户主目录。
/// `path` 不存在时返回 Ok(None)（幂等：调用方视为已完成）。
fn validate_delete_target(
    path: &str,
    pending_cleanup: Option<&std::path::Path>,
) -> Result<Option<std::path::PathBuf>, String> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err("待删除路径为空".into());
    }
    let dir = std::path::PathBuf::from(raw);
    if !dir.is_absolute() {
        return Err("待删除路径必须是绝对路径".into());
    }

    // 白名单：必须与登记的待清理目录一致（未迁移过或路径不符 → 拒绝）
    let Some(registered) = pending_cleanup else {
        return Err("当前没有待清理的旧数据目录".into());
    };
    let reg_canon = registered
        .canonicalize()
        .unwrap_or_else(|_| registered.to_path_buf());
    // 目标可能已被删除 → 用字符串比较兜底，避免因不存在而误判不符
    let matches = match dir.canonicalize() {
        Ok(c) => c == reg_canon,
        Err(_) => dir == registered,
    };
    if !matches {
        return Err("该路径不是本次迁移的旧数据目录，拒绝删除".into());
    }

    if !dir.is_dir() {
        return Ok(None);
    }
    let canon = dir.canonicalize().map_err(|e| e.to_string())?;

    // 独立红线：磁盘根 / 用户主目录（防登记值异常时误删）
    if canon.parent().is_none() {
        return Err("拒绝删除磁盘根目录".into());
    }
    if let Some(home) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from) {
        if home.canonicalize().map(|h| h == canon).unwrap_or(false) {
            return Err("拒绝删除用户主目录".into());
        }
    }
    Ok(Some(canon))
}

/// 删除迁移后的旧数据目录（用户在结果弹窗中关闭对话框时调用）。
///
/// 安全约束见 `validate_delete_target` —— 只接受 `pending_cleanup` 中登记的路径。
/// 删除成功后清空登记（幂等：重复调用会因「没有待清理目录」而拒绝，不会误删）。
#[tauri::command]
pub fn delete_old_data_dir(state: State<'_, Arc<AppState>>, path: String) -> CmdResult<()> {
    let pending = state.pending_cleanup.lock().ok().and_then(|g| g.clone());
    let target = validate_delete_target(&path, pending.as_deref()).map_err(BridgeError::Config)?;
    let Some(canon) = target else {
        // 已不存在视为成功（幂等：用户可能手动删过）
        tracing::info!(path = %path.trim(), "旧数据目录已不存在，跳过删除");
        if let Ok(mut slot) = state.pending_cleanup.lock() {
            *slot = None;
        }
        return Ok(());
    };
    std::fs::remove_dir_all(&canon).map_err(|e| BridgeError::Io(format!("删除失败：{e}")))?;
    if let Ok(mut slot) = state.pending_cleanup.lock() {
        *slot = None;
    }
    tracing::info!(path = %canon.display(), "旧数据目录已删除");
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
        "cancel" => AlertAction::Cancel,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 递归复制：含子目录、字节计数正确、data_dir.txt 被跳过（防自引用）。
    #[test]
    fn copy_dir_contents_recurses_and_skips_pointer() {
        let base = std::env::temp_dir().join(format!("dc-migrate-{}", std::process::id()));
        let src = base.join("src");
        let dst = base.join("dst");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(src.join("logs")).unwrap();
        std::fs::write(src.join("config.toml"), b"abc").unwrap();
        std::fs::write(src.join("logs").join("d.log"), b"de").unwrap();
        // 指针文件必须被跳过：它留在默认目录，复制过去会自引用
        std::fs::write(src.join("data_dir.txt"), b"ignored").unwrap();

        let (files, bytes) = copy_dir_contents(&src, &dst).unwrap();

        assert_eq!(files, 2, "应复制 2 个文件（跳过 data_dir.txt）");
        assert_eq!(bytes, 5);
        assert!(dst.join("config.toml").is_file());
        assert!(dst.join("logs").join("d.log").is_file());
        assert!(!dst.join("data_dir.txt").exists(), "指针不得被复制");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 空目录迁移：0 文件 0 字节，不报错。
    #[test]
    fn copy_dir_contents_empty_ok() {
        let base = std::env::temp_dir().join(format!("dc-migrate-empty-{}", std::process::id()));
        let src = base.join("src");
        let dst = base.join("dst");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&src).unwrap();

        let (files, bytes) = copy_dir_contents(&src, &dst).unwrap();

        assert_eq!((files, bytes), (0, 0));
        assert!(dst.is_dir(), "目标目录应被创建");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 覆盖写：目标已存在同名文件时以源为准（重复迁移幂等）。
    #[test]
    fn copy_dir_contents_overwrites() {
        let base = std::env::temp_dir().join(format!("dc-migrate-ow-{}", std::process::id()));
        let src = base.join("src");
        let dst = base.join("dst");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("config.toml"), b"new").unwrap();
        std::fs::write(dst.join("config.toml"), b"old-longer").unwrap();

        let (files, _) = copy_dir_contents(&src, &dst).unwrap();

        assert_eq!(files, 1);
        assert_eq!(std::fs::read(dst.join("config.toml")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 删除校验：白名单语义 —— 只有登记的待清理目录可删。
    ///
    /// 关键回归点：旧目录在迁移后**仍是本进程的 data_dir**（启动时解析、运行期不变），
    /// 若按「不得等于 data_dir」的黑名单校验，旧目录会被永久拒删（曾导致勾选无效）。
    #[test]
    fn validate_delete_whitelist_semantics() {
        let base = std::env::temp_dir().join(format!("dc-del-guard-{}", std::process::id()));
        let old_dir = base.join("old");
        let unrelated = base.join("unrelated");
        let _ = std::fs::remove_dir_all(&base);
        for d in [&old_dir, &unrelated] {
            std::fs::create_dir_all(d).unwrap();
        }

        let old_s = old_dir.display().to_string();
        let unrelated_s = unrelated.display().to_string();

        // 未登记任何待清理目录 → 一律拒绝
        assert!(
            validate_delete_target(&old_s, None).is_err(),
            "无登记时不得删除"
        );
        // 已登记 old_dir → old_dir 放行
        assert!(
            validate_delete_target(&old_s, Some(&old_dir)).is_ok(),
            "登记的旧目录应放行（这正是迁移后勾选删除的路径）"
        );
        // 已登记 old_dir → 其它目录仍拒绝
        assert!(
            validate_delete_target(&unrelated_s, Some(&old_dir)).is_err(),
            "未登记的目录不得删除"
        );
        // 空 / 相对路径
        assert!(validate_delete_target("", Some(&old_dir)).is_err());
        assert!(validate_delete_target("rel\\dir", Some(&old_dir)).is_err());

        // 登记的目录已不存在 → 幂等放行（Ok(None)）
        let gone = base.join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        let gone_s = gone.display().to_string();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(matches!(
            validate_delete_target(&gone_s, Some(&gone)),
            Ok(None)
        ));

        let _ = std::fs::remove_dir_all(&base);
    }
}
