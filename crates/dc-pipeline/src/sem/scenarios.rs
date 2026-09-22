//! 场景管理（§5.7 ScenarioStore 的场景半边；scenes.json）。
//!
//! 场景 = L1 规则过滤键（rules.json 的 `applies_to` / contacts.json 的 `profile`
//! 按 id 引用）+ 展示名 + **判定基线**（`base`，formal|casual）+ **判定阈值**（`threshold`）。
//! L2 的线性头按基线复用 —— 自定义场景不引入新的模型头，
//! `head.rs` 的双头结构因此不受场景数量影响。
//!
//! 阈值下沉到场景层（v1.2）：每个场景自带阈值，解除了同 `Profile` 场景共享阈值的限制。
//! 内置场景（`正式`/`个人`）不可修改、不可删除；自定义场景数量不限。
//! id 稳定（`s1`…），改名不影响既有规则引用；
//! 删除后残留的引用由 `base_of` 兜底回落 Formal（保守）。

use serde::{Deserialize, Serialize};

use super::doc::SceneDoc;
use super::rules::Profile;
use super::{THRESHOLD_CASUAL, THRESHOLD_FORMAL};

/// 单个场景。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// 稳定标识（rules/contacts 引用它；与展示名无关）
    pub id: String,
    /// 展示名（正式 / 个人 / 用户命名）
    pub name: String,
    /// 判定基线：线性头按此选择
    pub base: Profile,
    /// L2 判定阈值（§5.7）：score > threshold → Block，score ≤ threshold → Safe。
    pub threshold: f32,
    /// 内置场景（不可改、不可删）
    pub fixed: bool,
}

/// 内置场景（固定 id 与名称）。
pub const FORMAL_ID: &str = "formal";
pub const CASUAL_ID: &str = "casual";

/// 从 `base` 推导默认阈值，用于 `sanitize` 补齐旧文件缺省。
fn default_threshold_for_base(base: Profile) -> f32 {
    match base {
        Profile::Formal => THRESHOLD_FORMAL,
        Profile::Casual => THRESHOLD_CASUAL,
    }
}

fn builtin() -> Vec<Scenario> {
    vec![
        Scenario {
            id: FORMAL_ID.into(),
            name: "正式".into(),
            base: Profile::Formal,
            threshold: super::THRESHOLD_FORMAL,
            fixed: true,
        },
        Scenario {
            id: CASUAL_ID.into(),
            name: "个人".into(),
            base: Profile::Casual,
            threshold: super::THRESHOLD_CASUAL,
            fixed: true,
        },
    ]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SceneDef {
    pub id: String,
    pub name: String,
    pub base: Profile,
    /// 判定阈值；None = 旧文件缺省 → `sanitize` 用 base 补齐。
    #[serde(default)]
    pub threshold: Option<f32>,
}

/// 场景管理器：内置 + 自定义场景的唯一来源。
///
/// 持有 scenes.json 路径，增删改即持久化；其余模块（sem 判定、bridge 命令）
/// 只经 `all` / `get` / `base_of` / `threshold_of` / `name_of` 读参数，不感知存储细节。
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

    /// 从 scenes.json 加载；文件缺失/非法 → 仅内置场景（warn，不 Err——
    /// 场景文件损坏不应拖垮整个判定链路）。
    pub fn load(path: &std::path::Path) -> Self {
        let custom = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<SceneDoc>(&text) {
                Ok(parsed) => Self::sanitize(parsed.scene),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "scenes.json 非法，仅内置场景");
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

    /// 过滤非法条目：id 与内置冲突 / 空 id / 空名 / 重复 id → 丢弃（warn）。
    fn sanitize(scenes: Vec<SceneDef>) -> Vec<Scenario> {
        let builtin = builtin();
        let mut out = Vec::new();
        for s in scenes {
            let invalid = s.id.trim().is_empty()
                || s.name.trim().is_empty()
                || builtin.iter().any(|b| b.id == s.id)
                || out.iter().any(|e: &Scenario| e.id == s.id);
            if invalid {
                tracing::warn!(id = %s.id, name = %s.name, "scenes.json 含非法/重复场景条目，已忽略");
                continue;
            }
            out.push(Scenario {
                id: s.id,
                name: s.name.trim().to_string(),
                base: s.base,
                threshold: s.threshold.unwrap_or_else(|| default_threshold_for_base(s.base)),
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

    /// 场景 → L2 判定基线（线性头选择）；未知 id（含已删除场景的残留引用）回落 Formal（保守）。
    pub fn base_of(&self, id: &str) -> Profile {
        self.get(id).map(|s| s.base).unwrap_or(Profile::Formal)
    }

    /// 场景 → L2 判定阈值；未知 id 回退 0.55。
    pub fn threshold_of(&self, id: &str) -> f32 {
        self.get(id).map(|s| s.threshold).unwrap_or(THRESHOLD_FORMAL)
    }

    /// 展示名；未知 id 原样返回（reasons 文案兜底）。
    pub fn name_of(&self, id: &str) -> String {
        self.get(id)
            .map(|s| s.name)
            .unwrap_or_else(|| id.to_string())
    }

    /// 新增自定义场景（校验 + 分配 id + 持久化）。
    pub fn add(&mut self, name: &str, threshold: f32) -> Result<Scenario, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("场景名不能为空".into());
        }
        if self.all().iter().any(|s| s.name == name) {
            return Err(format!("场景名「{name}」已存在"));
        }
        let id = self.next_id();
        let s = Scenario {
            id: id.clone(),
            name: name.to_string(),
            base: Profile::Formal,
            threshold,
            fixed: false,
        };
        self.custom.push(s.clone());
        self.persist()?;
        tracing::info!(id = %id, name = %name, "场景已新增");
        Ok(s)
    }

    /// 修改自定义场景（内置场景拒绝；改名不影响 id 引用）。
    pub fn update(&mut self, id: &str, name: &str, threshold: f32) -> Result<Scenario, String> {
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
        s.threshold = threshold;
        let out = s.clone();
        self.persist()?;
        tracing::info!(id = %id, name = %name, "场景已修改");
        Ok(out)
    }

    /// 删除自定义场景（内置拒绝）。残留的规则/画像引用由 base_of 兜底。
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
        let scenes: Vec<SceneDef> = self
            .custom
            .iter()
            .map(|s| SceneDef {
                id: s.id.clone(),
                name: s.name.clone(),
                base: s.base,
                threshold: Some(s.threshold),
            })
            .collect();
        let doc = SceneDoc { scene: scenes };
        let text = serde_json::to_string_pretty(&doc)
            .map_err(|e| format!("scenes.json 序列化失败：{e}"))?;
        if self.path.as_os_str().is_empty() {
            return Ok(()); // builtin_only 占位实例（不落盘）
        }
        std::fs::write(&self.path, text).map_err(|e| format!("scenes.json 写入失败：{e}"))
    }
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
                threshold: THRESHOLD_FORMAL,
                fixed: true
            }
        );
        assert_eq!(
            all[1],
            Scenario {
                id: "casual".into(),
                name: "个人".into(),
                base: Profile::Casual,
                threshold: THRESHOLD_CASUAL,
                fixed: true
            }
        );
        assert!(m.is_known("formal") && m.is_known("casual"));
    }

    #[test]
    fn add_update_remove_roundtrip() {
        let path = tmp("crud");
        let mut m = ScenarioManager::load(&path);
        let s = m.add("工作群", THRESHOLD_FORMAL).unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(m.base_of("s1"), Profile::Formal);
        assert_eq!(m.threshold_of("s1"), THRESHOLD_FORMAL);

        // 重新加载 → 持久化生效
        let m2 = ScenarioManager::load(&path);
        assert_eq!(m2.get("s1").unwrap().name, "工作群");
        assert_eq!(m2.threshold_of("s1"), THRESHOLD_FORMAL);

        // 改名 + 阈值：id 不变（引用不受影响）
        let mut m = m2;
        let s = m.update("s1", "工作", THRESHOLD_CASUAL).unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(m.name_of("s1"), "工作");
        assert_eq!(m.base_of("s1"), Profile::Formal); // base 不变，仍然 Formal
        assert_eq!(m.threshold_of("s1"), THRESHOLD_CASUAL);

        // 删除后残留引用 → 兜底
        m.remove("s1").unwrap();
        assert!(!m.is_known("s1"));
        assert_eq!(m.base_of("s1"), Profile::Formal);
        assert_eq!(m.threshold_of("s1"), THRESHOLD_FORMAL);
        assert_eq!(m.name_of("s1"), "s1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fixed_scenarios_protected() {
        let path = tmp("fixed");
        let mut m = ScenarioManager::load(&path);
        assert!(m.update("formal", "正式2", THRESHOLD_CASUAL).is_err());
        assert!(m.update("casual", "个人2", THRESHOLD_FORMAL).is_err());
        assert!(m.remove("formal").is_err());
        assert!(m.remove("casual").is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn add_validates_name_and_id_reuse() {
        let path = tmp("limit");
        let mut m = ScenarioManager::load(&path);
        assert!(m.add("  ", THRESHOLD_FORMAL).is_err());
        assert!(m.add("正式", THRESHOLD_FORMAL).is_err(), "与内置重名拒绝");
        // 数量不限：连加多个都应成功
        for i in 0..12 {
            m.add(&format!("场景{i}"), THRESHOLD_CASUAL).unwrap();
        }
        assert_eq!(m.custom.len(), 12, "自定义场景数量不设上限");
        // id 分配：删除中间项后空洞可复用
        m.remove("s3").unwrap();
        let s = m.add("补位", THRESHOLD_FORMAL).unwrap();
        assert_eq!(s.id, "s3");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sanitize_drops_invalid_entries() {
        let path = tmp("sanitize");
        std::fs::write(
            &path,
            r#"{
  "scene": [
    {
      "id": "formal",
      "name": "冒充内置",
      "base": "casual"
    },
    {
      "id": "s1",
      "name": "正常",
      "base": "formal"
    },
    {
      "id": "s1",
      "name": "重复id",
      "base": "formal"
    }
  ]
}"#,
        )
        .unwrap();
        let m = ScenarioManager::load(&path);
        let all = m.all();
        assert_eq!(all.len(), 3, "内置 2 + 合法自定义 1");
        assert_eq!(all[2].name, "正常");
        // 旧文件缺少 threshold → serde default 从 base 推导
        assert_eq!(all[2].base, Profile::Formal);
        assert_eq!(all[2].threshold, THRESHOLD_FORMAL);
        let _ = std::fs::remove_file(&path);
    }
}
