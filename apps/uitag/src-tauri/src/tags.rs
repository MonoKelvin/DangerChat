use serde::{Deserialize, Serialize};

const DEFAULT_TAGS: &str = include_str!("tags.default.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagDef {
    pub name: String,
    pub label: String,
    pub color: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagsConfig {
    pub version: u32,
    pub tags: Vec<TagDef>,
}

#[derive(Debug, thiserror::Error)]
pub enum TagsError {
    #[error("tags.json 解析失败：{0}")]
    Parse(String),
    #[error("tags.json 不合法：{0}")]
    Invalid(String),
}

impl TagsConfig {
    /// 内置默认（微信四类区域，与主程序 config/tags.json 同 schema）。
    pub fn builtin() -> Self {
        Self::parse(DEFAULT_TAGS).expect("内嵌默认 tags 必须合法")
    }

    pub fn parse(text: &str) -> Result<Self, TagsError> {
        let cfg: TagsConfig =
            serde_json::from_str(text).map_err(|e| TagsError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), TagsError> {
        if self.version != 1 {
            return Err(TagsError::Invalid(format!("version 必须为 1，实际 {}", self.version)));
        }
        if self.tags.is_empty() {
            return Err(TagsError::Invalid("tags 不能为空".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for t in &self.tags {
            if t.name.is_empty() || !t.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(TagsError::Invalid(format!("标签名不合法：{:?}", t.name)));
            }
            if !seen.insert(t.name.as_str()) {
                return Err(TagsError::Invalid(format!("标签名重复：{}", t.name)));
            }
            if !valid_color(&t.color) {
                return Err(TagsError::Invalid(format!("颜色必须为 #RRGGBB：{}", t.color)));
            }
        }
        Ok(())
    }
}

fn valid_color(c: &str) -> bool {
    let b = c.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_default_is_valid_wechat_quads() {
        let cfg = TagsConfig::builtin();
        assert_eq!(cfg.version, 1);
        let names: Vec<&str> = cfg.tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["chat_list", "chat_window", "chat_target", "msg_input"]);
    }

    #[test]
    fn rejects_bad_version_dup_name_and_bad_color() {
        assert!(TagsConfig::parse(r#"{"version":2,"tags":[]}"#).is_err());
        let dup = "{\"version\":1,\"tags\":[\
            {\"name\":\"a\",\"label\":\"A\",\"color\":\"#111111\",\"enabled\":true},\
            {\"name\":\"a\",\"label\":\"B\",\"color\":\"#222222\",\"enabled\":true}]}";
        assert!(TagsConfig::parse(dup).is_err());
        let bad_color = "{\"version\":1,\"tags\":[\
            {\"name\":\"a\",\"label\":\"A\",\"color\":\"red\",\"enabled\":true}]}";
        assert!(TagsConfig::parse(bad_color).is_err());
    }
}
