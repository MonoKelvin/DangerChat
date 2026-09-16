//! 聊天对象画像（§5.7 ScenarioStore 的 contacts 半边；contacts.json，FR-SEM-03）。
//!
//! profile 值为场景 id（ScenarioManager 分发）；未标记对象回落 `formal`。
//! 解析不做场景合法性校验——已删除场景的残留条目由 `ScenarioManager::base_profile`
//! 兜底回落 Formal（保守），不因引用失效而拒绝整个画像文件。

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct TomlContacts {
    #[serde(default)]
    contact: Vec<ContactDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactDef {
    pub name: String,
    pub profile: String,
}

/// 画像表（不可变快照，热重载整体换入）。
#[derive(Debug, Clone, Default)]
pub struct ContactBook {
    entries: std::collections::HashMap<String, String>,
}

impl ContactBook {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let parsed: TomlContacts =
            toml::from_str(text).map_err(|e| format!("contacts.toml 解析失败：{e}"))?;
        let mut entries = std::collections::HashMap::new();
        for c in parsed.contact {
            if c.name.trim().is_empty() {
                return Err("联系人 name 不能为空".into());
            }
            if c.profile.trim().is_empty() {
                return Err(format!("联系人「{}」profile 不能为空", c.name));
            }
            entries.insert(c.name, c.profile);
        }
        Ok(Self { entries })
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let parsed: super::doc::ContactDoc =
            serde_json::from_str(text).map_err(|e| format!("contacts.json 解析失败：{e}"))?;
        let mut entries = std::collections::HashMap::new();
        for c in parsed.contact {
            if c.name.trim().is_empty() {
                return Err("联系人 name 不能为空".into());
            }
            if c.profile.trim().is_empty() {
                return Err(format!("联系人「{}」profile 不能为空", c.name));
            }
            entries.insert(c.name, c.profile);
        }
        Ok(Self { entries })
    }

    /// 对象画像（场景 id）；**未标记一律 formal（保守）**。
    pub fn profile_of(&self, name: &str) -> &str {
        self.entries
            .get(name)
            .map(String::as_str)
            .unwrap_or("formal")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmarked_defaults_to_formal() {
        let book = ContactBook::default();
        assert_eq!(book.profile_of("完全未标记的人"), "formal");
    }

    #[test]
    fn marked_profiles() {
        let book = ContactBook::from_toml(
            r#"
[[contact]]
name = "文件传输助手"
profile = "casual"

[[contact]]
name = "张总"
profile = "formal"

[[contact]]
name = "项目群"
profile = "s1"
"#,
        )
        .unwrap();
        assert_eq!(book.profile_of("文件传输助手"), "casual");
        assert_eq!(book.profile_of("张总"), "formal");
        assert_eq!(book.profile_of("项目群"), "s1");
        assert_eq!(book.profile_of("别的谁"), "formal");
    }

    #[test]
    fn empty_profile_rejected_but_unknown_id_tolerated() {
        // 未知场景 id（如已删除的场景）不拒绝解析——判定时兜底 formal
        let ok = ContactBook::from_toml(
            r#"[[contact]]
name = "x"
profile = "gone-scene"
"#,
        );
        assert!(ok.is_ok());

        let bad = ContactBook::from_toml(
            r#"[[contact]]
name = "x"
profile = ""
"#,
        );
        assert!(bad.is_err());
    }
}
