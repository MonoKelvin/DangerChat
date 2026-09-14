//! 聊天对象画像（§5.7 ScenarioStore 的 contacts 半边；contacts.toml，FR-SEM-03）。
//!
//! 未标记对象按 **formal** 保守处理（§5.7 判定算法）。

use serde::Deserialize;

use super::rules::Profile;

#[derive(Debug, Deserialize)]
struct TomlContacts {
    #[serde(default)]
    contact: Vec<ContactDef>,
}

#[derive(Debug, Deserialize)]
struct ContactDef {
    name: String,
    profile: String,
}

/// 画像表（不可变快照，热重载整体换入）。
#[derive(Debug, Clone, Default)]
pub struct ContactBook {
    entries: std::collections::HashMap<String, Profile>,
}

impl ContactBook {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let parsed: TomlContacts =
            toml::from_str(text).map_err(|e| format!("contacts.toml 解析失败：{e}"))?;
        let mut entries = std::collections::HashMap::new();
        for c in parsed.contact {
            let profile = match c.profile.as_str() {
                "formal" => Profile::Formal,
                "casual" => Profile::Casual,
                other => return Err(format!("画像非法「{other}」（formal|casual）")),
            };
            if c.name.trim().is_empty() {
                return Err("联系人 name 不能为空".into());
            }
            entries.insert(c.name, profile);
        }
        Ok(Self { entries })
    }

    /// 对象画像；**未标记一律 Formal（保守）**。
    pub fn profile_of(&self, name: &str) -> Profile {
        self.entries
            .get(name)
            .copied()
            .unwrap_or(Profile::Formal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmarked_defaults_to_formal() {
        let book = ContactBook::default();
        assert_eq!(book.profile_of("完全未标记的人"), Profile::Formal);
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
"#,
        )
        .unwrap();
        assert_eq!(book.profile_of("文件传输助手"), Profile::Casual);
        assert_eq!(book.profile_of("张总"), Profile::Formal);
        assert_eq!(book.profile_of("别的谁"), Profile::Formal);
    }

    #[test]
    fn invalid_profile_rejected() {
        let bad = ContactBook::from_toml(r#"[[contact]]
name = "x"
profile = "other"
"#);
        assert!(bad.is_err());
    }
}
