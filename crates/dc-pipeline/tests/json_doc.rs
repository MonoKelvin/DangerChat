//! JSON 文档序列化测试

use dc_pipeline::sem::contacts::ContactDef;
use dc_pipeline::sem::doc::{ContactDoc, RuleDoc, SceneDoc};
use dc_pipeline::sem::rules::Profile;
use dc_pipeline::sem::rules::{MatchKind, RuleDef};
use dc_pipeline::sem::scenarios::SceneDef;

#[test]
fn scene_doc_json_roundtrip() {
    let doc = SceneDoc {
        scene: vec![
            SceneDef {
                id: "s1".into(),
                name: "工作".into(),
                base: Profile::Formal,
            },
            SceneDef {
                id: "s2".into(),
                name: "朋友".into(),
                base: Profile::Casual,
            },
        ],
    };

    let json = serde_json::to_string_pretty(&doc).expect("序列化失败");
    let parsed: SceneDoc = serde_json::from_str(&json).expect("反序列化失败");

    assert_eq!(parsed.scene.len(), 2);
    assert_eq!(parsed.scene[0].id, "s1");
    assert_eq!(parsed.scene[1].name, "朋友");
}

#[test]
fn rule_doc_json_roundtrip() {
    let doc = RuleDoc {
        rule: vec![RuleDef {
            pattern: "敏感词".into(),
            r#match: MatchKind::Word,
            applies_to: vec!["all".into()],
        }],
    };

    let json = serde_json::to_string_pretty(&doc).expect("序列化失败");
    let parsed: RuleDoc = serde_json::from_str(&json).expect("反序列化失败");

    assert_eq!(parsed.rule.len(), 1);
    assert_eq!(parsed.rule[0].pattern, "敏感词");
}

#[test]
fn contact_doc_json_roundtrip() {
    let doc = ContactDoc {
        contact: vec![
            ContactDef {
                name: "张三".into(),
                profile: "formal".into(),
            },
            ContactDef {
                name: "李四".into(),
                profile: "casual".into(),
            },
        ],
    };

    let json = serde_json::to_string_pretty(&doc).expect("序列化失败");
    let parsed: ContactDoc = serde_json::from_str(&json).expect("反序列化失败");

    assert_eq!(parsed.contact.len(), 2);
    assert_eq!(parsed.contact[0].name, "张三");
    assert_eq!(parsed.contact[1].profile, "casual");
}
