//! 场景管理（§5.7 ScenarioStore 的场景半边；scenes.toml）。
//!
//! 场景 = L1 规则过滤键（rules.toml 的 `applies_to` / contacts.toml 的 `profile`
//! 按 id 引用）+ 展示名 + **判定基线**（`base`，formal|casual）。
//! L2 的阈值与线性头按基线复用 —— 自定义场景不引入新的模型头，
//! `head.rs` 的双头结构因此不受场景数量影响。
//!
//! 内置场景（`正式`/`个人`）不可修改、不可删除；自定义场景最多补到
//! 总数 10（`MAX_SCENARIOS`）。id 稳定（`s1`…），改名不影响既有规则引用；
//! 删除后残留的引用由 `base_profile` 兜底回落 Formal（保守）。

use serde::{Deserialize, Serialize};

use super::rules::Profile;

/// 场景总数上限（内置 2 + 自定义最多 8）。
pub const MAX_SCENARIOS: usize = 10;

/// 单个场景。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// 稳定标识（rules/contacts 引用它；与展示名无关）
    pub id: String,
    /// 展示名（正式 / 个人 / 用户命名）
    pub name: String,
    /// 判定基线：L2 阈值与线性头按它选取
    pub base: Profile,
    /// 内置场景（不可改、不可删）
    pub fixed: bool,
}

/// 内置场景（固定 id 与名称）。
pub const FORMAL_ID: &str = "formal";
pub const CASUAL_ID: &str = "casual";

fn builtin() -> Vec<Scenario> {
    vec![
        Scenario {
            id: FORMAL_ID.into(),
            name: "正式".into(),
            base: Profile::Formal,
            fixed: true,
        },
        Scenario {
            id: CASUAL_ID.into(),
            name: "个人".into(),
            base: Profile::Casual,
            fixed: true,
        },
    ]
}

#[derive(Debug, Deserialize)]
struct TomlScenes {
    #[serde(default)]
    scene: Vec<TomlScene>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TomlScene {
    id: String,
    name: String,
    base: Profile,
}

/// 场景管理器：内置 + 自定义场景的唯一来源。
///
/// 持有 scenes.toml 路径，增删改即持久化；其余模块（sem 判定、bridge 命令）
/// 只经 `all` / `get` / `base_profile` / `name_of` 读参数，不感知存储细节。
pub struct ScenarioManager {
    path: std::path::PathBuf,
    custom: Vec<Scenario>,
}

impl ScenarioManager {
    /// 仅内置场景（SemStage::empty/l1_only 占位；无持久化路径，不做写操作）。
    pub fn builtin_only() -> Self {
        Self {
            path: std::path::PathBuf::new(),
            custom: Vec::new(),
        }
    }

    /// 从 scenes.toml 加载；文件缺失/非法 → 仅内置场景（warn，不 Err——
    /// 场景文件损坏不应拖垮整个判定链路）。
    pub fn load(path: &std::path::Path) -> Self {
        let custom = match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<TomlScenes>(&text) {
                Ok(parsed) => Self::sanitize(parsed.scene),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "scenes.toml 非法，仅内置场景");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self {
            path: path.to_path_buf(),
            custom,
        }
    }

    /// 过滤非法条目：id 与内置冲突 / 空 id / 空名 / 重名 → 丢弃（warn）。
    fn sanitize(scenes: Vec<TomlScene>) -> Vec<Scenario> {
        let builtin = builtin();
        let mut out = Vec::new();
        for s in scenes {
            let invalid = s.id.trim().is_empty()
                || s.name.trim().is_empty()
                || builtin
                    .iter()
                    .any(|b| b.id == s.id || b.name == s.name.trim())
                || out
                    .iter()
                    .any(|e: &Scenario| e.id == s.id || e.name == s.name.trim());
            if invalid {
                tracing::warn!(id = %s.id, "scenes.toml 含非法/重复场景条目，已忽略");
                continue;
            }
            out.push(Scenario {
                id: s.id,
                name: s.name.trim().to_string(),
                base: s.base,
                fixed: false,
            });
        }
        out
    }

    /// 全部场景（内置在前）。
    pub fn all(&self) -> Vec<Scenario> {
        let mut v = builtin();
        v.extend(self.custom.iter().cloned());
        v
    }

    pub fn get(&self, id: &str) -> Option<Scenario> {
        self.all().into_iter().find(|s| s.id == id)
    }

    /// id 是否存在（含内置）。
    pub fn is_known(&self, id: &str) -> bool {
        id == FORMAL_ID || id == CASUAL_ID || self.custom.iter().any(|s| s.id == id)
    }

    /// 场景 → L2 判定基线；未知 id（含已删除场景的残留引用）回落 Formal（保守）。
    pub fn base_profile(&self, id: &str) -> Profile {
        self.get(id).map(|s| s.base).unwrap_or(Profile::Formal)
    }

    /// 展示名；未知 id 原样返回（reasons 文案兜底）。
    pub fn name_of(&self, id: &str) -> String {
        self.get(id)
            .map(|s| s.name)
            .unwrap_or_else(|| id.to_string())
    }

    /// 新增自定义场景（校验 + 分配 id + 持久化）。
    pub fn add(&mut self, name: &str, base: Profile) -> Result<Scenario, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("场景名不能为空".into());
        }
        if self.all().iter().any(|s| s.name == name) {
            return Err(format!("场景名「{name}」已存在"));
        }
        if self.all().len() >= MAX_SCENARIOS {
            return Err(format!("场景总数已达上限 {MAX_SCENARIOS}"));
        }
        let id = self.next_id();
        let s = Scenario {
            id: id.clone(),
            name: name.to_string(),
            base,
            fixed: false,
        };
        self.custom.push(s.clone());
        self.persist()?;
        tracing::info!(id = %id, name = %name, "场景已新增");
        Ok(s)
    }

    /// 修改自定义场景（内置场景拒绝；改名不影响 id 引用）。
    pub fn update(&mut self, id: &str, name: &str, base: Profile) -> Result<Scenario, String> {
        let Some(idx) = self.custom.iter().position(|s| s.id == id) else {
            return Err(format!("场景不存在或为内置场景：{id}"));
        };
        let name = name.trim();
        if name.is_empty() {
            return Err("场景名不能为空".into());
        }
        // 重名检查先于可变借用：经 all() 快照比较，避免双重借用
        if self.all().iter().any(|o| o.name == name && o.id != id) {
            return Err(format!("场景名「{name}」已存在"));
        }
        let s = &mut self.custom[idx];
        s.name = name.to_string();
        s.base = base;
        let out = s.clone();
        self.persist()?;
        tracing::info!(id = %id, name = %name, "场景已修改");
        Ok(out)
    }

    /// 删除自定义场景（内置拒绝）。残留的规则/画像引用由 base_profile 兜底。
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if id == FORMAL_ID || id == CASUAL_ID {
            return Err("内置场景不可删除".into());
        }
        let before = self.custom.len();
        self.custom.retain(|s| s.id != id);
        if self.custom.len() == before {
            return Err(format!("场景不存在：{id}"));
        }
        self.persist()?;
        tracing::info!(id = %id, "场景已删除");
        Ok(())
    }

    /// 分配最小可用 id（s1、s2…；删除后的空洞可复用）。
    fn next_id(&self) -> String {
        for n in 1.. {
            let id = format!("s{n}");
            if !self.custom.iter().any(|s| s.id == id) {
                return id;
            }
        }
        unreachable!()
    }

    fn persist(&self) -> Result<(), String> {
        let scenes: Vec<TomlScene> = self
            .custom
            .iter()
            .map(|s| TomlScene {
                id: s.id.clone(),
                name: s.name.clone(),
                base: s.base,
            })
            .collect();
        let text = toml::to_string_pretty(&TomlScenesSerialize { scene: scenes })
            .map_err(|e| format!("scenes.toml 序列化失败：{e}"))?;
        if self.path.as_os_str().is_empty() {
            return Ok(()); // builtin_only 占位实例（不落盘）
        }
        std::fs::write(&self.path, text).map_err(|e| format!("scenes.toml 写入失败：{e}"))
    }
}

#[derive(Debug, Serialize)]
struct TomlScenesSerialize {
    scene: Vec<TomlScene>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("dc-scenes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn builtin_scenarios_present() {
        let m = ScenarioManager::builtin_only();
        let all = m.all();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0],
            Scenario {
                id: "formal".into(),
                name: "正式".into(),
                base: Profile::Formal,
                fixed: true
            }
        );
        assert_eq!(
            all[1],
            Scenario {
                id: "casual".into(),
                name: "个人".into(),
                base: Profile::Casual,
                fixed: true
            }
        );
        assert!(m.is_known("formal") && m.is_known("casual"));
    }

    #[test]
    fn add_update_remove_roundtrip() {
        let path = tmp("crud");
        let mut m = ScenarioManager::load(&path);
        let s = m.add("工作群", Profile::Formal).unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(m.base_profile("s1"), Profile::Formal);

        // 重新加载 → 持久化生效
        let m2 = ScenarioManager::load(&path);
        assert_eq!(m2.get("s1").unwrap().name, "工作群");

        // 改名：id 不变（引用不受影响）
        let mut m = m2;
        let s = m.update("s1", "工作", Profile::Casual).unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(m.name_of("s1"), "工作");
        assert_eq!(m.base_profile("s1"), Profile::Casual);

        // 删除后残留引用 → 兜底 Formal
        m.remove("s1").unwrap();
        assert!(!m.is_known("s1"));
        assert_eq!(m.base_profile("s1"), Profile::Formal);
        assert_eq!(m.name_of("s1"), "s1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fixed_scenarios_protected() {
        let path = tmp("fixed");
        let mut m = ScenarioManager::load(&path);
        assert!(m.update("formal", "正式2", Profile::Casual).is_err());
        assert!(m.update("casual", "个人2", Profile::Formal).is_err());
        assert!(m.remove("formal").is_err());
        assert!(m.remove("casual").is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn add_validates_name_and_limit() {
        let path = tmp("limit");
        let mut m = ScenarioManager::load(&path);
        assert!(m.add("  ", Profile::Formal).is_err());
        assert!(m.add("正式", Profile::Formal).is_err(), "与内置重名拒绝");
        // 补满到 10：内置 2 + 自定义 8
        for i in 0..8 {
            m.add(&format!("场景{i}"), Profile::Casual).unwrap();
        }
        assert!(m.add("第十一个", Profile::Formal).is_err(), "总数上限 10");
        // id 分配：删除中间项后空洞可复用
        m.remove("s3").unwrap();
        let s = m.add("补位", Profile::Formal).unwrap();
        assert_eq!(s.id, "s3");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sanitize_drops_invalid_entries() {
        let path = tmp("sanitize");
        std::fs::write(
            &path,
            r#"
[[scene]]
id = "formal"
name = "冒充内置"
base = "casual"

[[scene]]
id = "s1"
name = "正常"
base = "formal"

[[scene]]
id = "s1"
name = "重复id"
base = "formal"
"#,
        )
        .unwrap();
        let m = ScenarioManager::load(&path);
        let all = m.all();
        assert_eq!(all.len(), 3, "内置 2 + 合法自定义 1");
        assert_eq!(all[2].name, "正常");
        let _ = std::fs::remove_file(&path);
    }
}
