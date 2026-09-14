//! L1 规则引擎（§5.7 RuleEngine；rules.toml，FR-SEM-04）。
//!
//! 三种匹配模式：
//! - `word`：CJK 逐字匹配 + 拉丁词边界（`sb` 不命中 `absb`）
//! - `substring`：子串包含
//! - `regex`：正则（regex crate，RE2 语法子集）
//!
//! 字面量（word/substring）合并进一个 AhoCorasick 自动机，一次扫描出全部命中；
//! 加载后零 I/O。热重载 = 重建 RuleSet 整体换入。

use aho_corasick::AhoCorasick;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// 场景画像（contacts.toml 的 profile 值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Formal,
    Casual,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Formal => "formal",
            Profile::Casual => "casual",
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 严重级别（rules.toml 的 severity）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warn,
    Block,
}

impl From<Severity> for crate::verdict::VerdictLevel {
    fn from(s: Severity) -> Self {
        match s {
            Severity::Warn => crate::verdict::VerdictLevel::Warn,
            Severity::Block => crate::verdict::VerdictLevel::Block,
        }
    }
}

/// rules.toml 的 [[rule]] 条目（反序列化形态）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleDef {
    pub pattern: String,
    /// word | substring | regex
    pub r#match: MatchKind,
    pub severity: Severity,
    /// formal | casual | all（小写）
    #[serde(default = "default_applies")]
    pub applies_to: Vec<String>,
}

fn default_applies() -> Vec<String> {
    vec!["all".into()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    Word,
    Substring,
    Regex,
}

/// 命中结果。
#[derive(Debug, Clone, PartialEq)]
pub struct RuleHit {
    pub pattern: String,
    pub severity: Severity,
}

/// 编译后的规则集（不可变，加载后零 I/O）。
pub struct RuleSet {
    /// 字面量自动机（word + substring 合并）；pattern 索引回表。
    literal_ac: AhoCorasick,
    /// 字面量元数据（与自动机 pattern 同序）：(def, kind)
    literals: Vec<(RuleDef, MatchKind)>,
    regexes: Vec<RuleDef>,
}

impl RuleSet {
    pub fn from_defs(defs: Vec<RuleDef>) -> Result<Self, String> {
        let mut literals = Vec::new();
        let mut patterns = Vec::new();
        let mut regexes = Vec::new();
        for d in defs {
            if d.pattern.is_empty() {
                return Err("规则 pattern 不能为空".into());
            }
            match d.r#match {
                MatchKind::Regex => {
                    Regex::new(&d.pattern).map_err(|e| format!("正则非法「{}」：{e}", d.pattern))?;
                    regexes.push(d);
                }
                _ => {
                    patterns.push(d.pattern.clone());
                    literals.push((d.clone(), d.r#match));
                }
            }
        }
        let literal_ac = AhoCorasick::new(&patterns).map_err(|e| format!("字面量自动机构建失败：{e}"))?;
        Ok(Self {
            literal_ac,
            literals,
            regexes,
        })
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let parsed: TomlRules = toml::from_str(text).map_err(|e| format!("rules.toml 解析失败：{e}"))?;
        Self::from_defs(parsed.rule)
    }

    pub fn is_empty(&self) -> bool {
        self.literals.is_empty() && self.regexes.is_empty()
    }

    fn applies(&self, def: &RuleDef, profile: Profile) -> bool {
        def.applies_to.iter().any(|a| a == "all" || a == profile.as_str())
    }

    /// 在文本中找第一条命中的规则（**block 优先于 warn**——同一文本命中多条时
    /// 取最严重的；同级取先出现的）。
    pub fn first_hit(&self, text: &str, profile: Profile) -> Option<RuleHit> {
        if text.is_empty() {
            return None;
        }
        let mut best: Option<RuleHit> = None;
        let mut consider = |hit: RuleHit| {
            let better = match &best {
                None => true,
                Some(b) => {
                    hits_block(hit.severity) && !hits_block(b.severity)
                }
            };
            if better {
                best = Some(hit);
            }
        };

        // 字面量：一次扫描全部命中，逐个校验 word 边界与适用场景
        for m in self.literal_ac.find_iter(text) {
            let (def, kind) = &self.literals[m.pattern().as_usize()];
            if !self.applies(def, profile) {
                continue;
            }
            let matched = match kind {
                MatchKind::Word => word_bounded(text, m.start(), m.end()),
                _ => true, // substring
            };
            if matched {
                consider(RuleHit {
                    pattern: def.pattern.clone(),
                    severity: def.severity,
                });
            }
        }

        for def in &self.regexes {
            if !self.applies(def, profile) {
                continue;
            }
            // 只需 is_match；块级语义已由 block-priority 汇聚
            if Regex::new(&def.pattern).ok().is_some_and(|re| re.is_match(text)) {
                consider(RuleHit {
                    pattern: def.pattern.clone(),
                    severity: def.severity,
                });
            }
        }
        best
    }
}

fn hits_block(s: Severity) -> bool {
    s == Severity::Block
}

/// 词边界判定：拉丁字母/数字两侧不能是同族字符；CJK 无边界概念恒真。
fn word_bounded(text: &str, start: usize, end: usize) -> bool {
    let bytes = text.as_bytes();
    let lat = |b: u8| b.is_ascii_alphanumeric();
    let before_ok = start == 0 || bytes
        .get(start - 1)
        .is_none_or(|&b| !lat(b) && b != b'_');
    let after_ok = end >= bytes.len() || bytes.get(end).is_none_or(|&b| !lat(b) && b != b'_');
    before_ok && after_ok
}

#[derive(Debug, Deserialize)]
struct TomlRules {
    #[serde(default)]
    rule: Vec<RuleDef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> RuleSet {
        RuleSet::from_defs(vec![
            RuleDef {
                pattern: "sb".into(),
                r#match: MatchKind::Word,
                severity: Severity::Block,
                applies_to: vec!["formal".into()],
            },
            RuleDef {
                pattern: "(傻|沙)(比|逼|雕)".into(),
                r#match: MatchKind::Regex,
                severity: Severity::Warn,
                applies_to: vec!["formal".into()],
            },
            RuleDef {
                pattern: "卧槽".into(),
                r#match: MatchKind::Substring,
                severity: Severity::Warn,
                applies_to: vec!["all".into()],
            },
        ])
        .unwrap()
    }

    #[test]
    fn word_boundary() {
        let rs = rules();
        // word 模式：sb 不命中嵌入词
        assert!(rs.first_hit("这个人真是 sb", Profile::Formal).is_some());
        assert!(rs.first_hit("absb 内嵌", Profile::Formal).is_none());
        // CJK 两侧无边界要求
        assert!(rs.first_hit("你个 sb东西", Profile::Formal).is_some());
    }

    #[test]
    fn profile_filter() {
        let rs = rules();
        // sb 只适用 formal；casual 场景不命中
        assert!(rs.first_hit("你是 sb", Profile::Casual).is_none());
        // 卧槽 applies_to=all 两场景都命中
        assert!(rs.first_hit("卧槽", Profile::Casual).is_some());
    }

    #[test]
    fn block_priority_over_warn() {
        let rs = RuleSet::from_defs(vec![
            RuleDef {
                pattern: "卧槽".into(),
                r#match: MatchKind::Substring,
                severity: Severity::Warn,
                applies_to: default_applies(),
            },
            RuleDef {
                pattern: "sb".into(),
                r#match: MatchKind::Word,
                severity: Severity::Block,
                applies_to: default_applies(),
            },
        ])
        .unwrap();
        let hit = rs.first_hit("卧槽这个 sb", Profile::Formal).unwrap();
        assert_eq!(hit.severity, Severity::Block);
        assert_eq!(hit.pattern, "sb");
    }

    #[test]
    fn regex_match() {
        let rs = rules();
        assert!(rs.first_hit("你别傻逼了", Profile::Formal).is_some());
        assert!(rs.first_hit("你别傻雕了", Profile::Formal).is_some());
        assert!(rs.first_hit("你真傻", Profile::Formal).is_none());
    }

    #[test]
    fn toml_roundtrip_and_bad_regex() {
        let toml = r#"
[[rule]]
pattern = "sb"
match = "word"
severity = "block"
applies_to = ["formal"]

[[rule]]
pattern = "(傻|沙)(比|逼)"
match = "regex"
severity = "warn"
applies_to = ["all"]
"#;
        let rs = RuleSet::from_toml(toml).unwrap();
        assert_eq!(rs.literals.len(), 1);
        assert_eq!(rs.regexes.len(), 1);

        let bad = RuleSet::from_toml(r#"[[rule]]
pattern = "(unclosed"
match = "regex"
severity = "warn"
"#);
        assert!(bad.is_err());
    }

    #[test]
    fn empty_text_no_hit() {
        let rs = rules();
        assert!(rs.first_hit("", Profile::Formal).is_none());
    }
}
