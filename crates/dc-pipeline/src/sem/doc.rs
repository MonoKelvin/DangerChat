//! JSON 文档类型（rules.json / contacts.json / scenes.json 的顶层结构）。
//!
//! 迁移设计：
//! - rules.toml `[[rule]]` → rules.json `{"rule": [...]}`
//! - contacts.toml `[[contact]]` → contacts.json `{"contact": [...]}`
//! - scenes.toml `[[scene]]` → scenes.json `{"scene": [...]}`
//!
//! 顶层包一层对象，为未来扩展预留字段空间（如 `version` / `metadata`）。

use serde::{Deserialize, Serialize};

use super::contacts::ContactDef;
use super::rules::RuleDef;
use super::scenarios::SceneDef;

/// rules.json 顶层结构
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleDoc {
    #[serde(default)]
    pub rule: Vec<RuleDef>,
}

/// contacts.json 顶层结构
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactDoc {
    #[serde(default)]
    pub contact: Vec<ContactDef>,
}

/// scenes.json 顶层结构
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SceneDoc {
    #[serde(default)]
    pub scene: Vec<SceneDef>,
}
