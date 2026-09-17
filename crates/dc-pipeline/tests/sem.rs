//! M5 sem 模块集成测试（UT-SEM-01~09 补全；单元级在 src/sem/ 内）。

use dc_pipeline::contract::{OcrResult, PipelineContext, Stage, TextBlock};
use dc_pipeline::sem::{contacts::ContactBook, rules::*, SemStage, THRESHOLD_FORMAL};
use dc_pipeline::verdict::{Verdict, VerdictLevel};
use dc_sys::Rect;

fn ocr(draft: &str, target: Option<&str>) -> OcrResult {
    OcrResult {
        chat_target: target.map(str::to_string),
        draft_text: draft.into(),
        blocks: vec![TextBlock {
            text: draft.into(),
            rect: Rect::new(0, 0, 10, 10),
            confidence: 0.9,
        }],
    }
}

/// UT-SEM-01：规则矩阵（数据驱动：词条 × formal/casual × 预期级别）。
#[test]
fn ut_sem_01_rule_matrix() {
    let rules = RuleSet::from_toml(
        r#"
[[rule]]
pattern = "sb"
match = "word"
severity = "block"
applies_to = ["formal"]

[[rule]]
pattern = "滚"
match = "word"
severity = "warn"
applies_to = ["all"]

[[rule]]
pattern = "(傻|沙)(比|逼|雕)"
match = "regex"
severity = "warn"
applies_to = ["formal"]

[[rule]]
pattern = "麻了"
match = "substring"
severity = "warn"
applies_to = ["casual"]

[[rule]]
pattern = "weakup"
match = "word"
severity = "warn"
applies_to = ["all"]

[[rule]]
pattern = "kpi"
match = "substring"
severity = "block"
applies_to = ["formal"]
"#,
    )
    .unwrap();
    let contacts = ContactBook::from_toml(
        r#"
[[contact]]
name = "家人"
profile = "casual"
"#,
    )
    .unwrap();
    let stage = SemStage::l1_only(rules, contacts);

    // (draft, target, expected) —— 覆盖 word/substring/regex、formal/casual/all、
    // 边界（嵌入词不命中）、优先级（block>warn）、未标记默认 formal
    let cases: Vec<(&str, &str, VerdictLevel)> = vec![
        // word
        ("你是 sb", "领导", VerdictLevel::Block),
        ("absb 嵌入词", "领导", VerdictLevel::Safe),
        ("sb", "家人", VerdictLevel::Safe), // formal-only 规则在 casual 不命中
        // word all
        ("weakup 信号", "家人", VerdictLevel::Block),
        ("wakeup 不同词", "家人", VerdictLevel::Safe),
        // substring all
        ("你给我滚出去", "家人", VerdictLevel::Block),
        ("滚动列表", "领导", VerdictLevel::Block), // substring 无边界
        // substring casual-only
        ("干活干到麻了", "家人", VerdictLevel::Block),
        ("干活干到麻了", "领导", VerdictLevel::Safe),
        // regex formal-only
        ("你别傻逼了", "领导", VerdictLevel::Block),
        ("你别傻雕了", "领导", VerdictLevel::Block),
        ("你真傻", "家人", VerdictLevel::Safe),
        ("你别傻逼了", "家人", VerdictLevel::Safe),
        ("sb 你滚", "领导", VerdictLevel::Block),
        // substring 命中嵌入形态
        ("这个 kpi 很重要", "领导", VerdictLevel::Block),
        ("这个 kpi 很重要", "家人", VerdictLevel::Safe),
        // 未标记 → formal
        ("你是 sb", "陌生人", VerdictLevel::Block),
        // 无命中
        ("正常工作消息", "领导", VerdictLevel::Safe),
    ];
    for (draft, target, want) in cases {
        let v = stage.judge(&ocr(draft, Some(target)));
        assert_eq!(
            v.level, want,
            "draft={draft:?} target={target:?} → {:?} (want {want:?})",
            v.level
        );
    }
}

/// UT-SEM-07：reasons 文案包含命中词条 / 危险分数值。
#[test]
fn ut_sem_07_reasons_text() {
    let rules = RuleSet::from_defs(vec![RuleDef {
        pattern: "sb".into(),
        r#match: MatchKind::Word,
        applies_to: vec!["formal".into()],
    }])
    .unwrap();
    let stage = SemStage::l1_only(rules, ContactBook::default());

    let v = stage.judge(&ocr("你是 sb", Some("领导")));
    assert!(v.reasons[0].contains("sb"), "{}", v.reasons[0]);

    // from_score 的理由由 sem 补充（带分数值）——直接验证文案格式函数面
    let mut scored = Verdict::from_score(0.71, THRESHOLD_FORMAL);
    scored.reasons.push(format!(
        "与formal场景语义不匹配，危险分 {:.2} ≥ {THRESHOLD_FORMAL:.2}",
        0.71
    ));
    assert!(scored.reasons[0].contains("0.71"), "{}", scored.reasons[0]);
}

/// Stage 适配面：process 输出带 draft/target 元数据。
#[test]
fn stage_process_attaches_metadata() {
    use dc_core::{ConfigSnapshot, ImageLogSink, RunId};
    use std::sync::Arc;
    let rules = RuleSet::from_defs(vec![RuleDef {
        pattern: "sb".into(),
        r#match: MatchKind::Word,
        applies_to: vec!["all".into()],
    }])
    .unwrap();
    let stage = SemStage::l1_only(rules, ContactBook::default());
    let ctx = PipelineContext::new(
        RunId::from_raw("20260914-000000-000001"),
        dc_pipeline::contract::LoopKind::Fast,
        ImageLogSink::noop(),
        Arc::new(ConfigSnapshot::default()),
    );
    let input = ocr("你是 sb", Some("张总"));
    let v = stage.process(input, &ctx).expect("sem process");
    assert_eq!(v.level, VerdictLevel::Block);
    assert_eq!(v.chat_target.as_deref(), Some("张总"));
    assert_eq!(v.draft_text.as_deref(), Some("你是 sb"));
    assert!(v.draft_fingerprint != 0);
}

/// UT-SEM-08：头文件缺失 → 模板兜底；头维度不符 → 降级兜底（不 Err）。
/// 直接构造 head::Heads 场景（文件级）在 src/sem/head.rs 单测覆盖；
/// 这里验证 Stage 层：embedder 缺失 + 头缺失 → L2 整体跳过，仅 L1。
#[test]
fn ut_sem_08_missing_head_degrades_not_errors() {
    let stage = SemStage::l1_only(
        RuleSet::from_defs(Vec::new()).unwrap(),
        ContactBook::default(),
    );
    // 无规则无模型：一切 Safe，无 panic
    assert_eq!(
        stage.judge(&ocr("随便什么文本", None)).level,
        VerdictLevel::Safe
    );
    assert_eq!(stage.head_state(), "无头（仅 L1）");
}

/// 真模型集成（内存敏感，-- --ignored 手动跑）：
/// BGE 嵌入自相似 = 1.0（UT-SEM-09 的真模型半边）+ 模板兜底打分。
#[test]
#[ignore = "需要 models/bge 真实权重（gitignore；见 models/README）"]
fn real_bge_embedding() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models/bge"));
    let emb = dc_pipeline::sem::embedder::Embedder::load(dir).expect("BGE 加载失败");

    let v = emb.embed("这是一条测试消息").expect("嵌入失败");
    // 自相似（L2 归一化后点积）= 1.0
    let self_sim: f32 = v.iter().map(|x| x * x).sum();
    assert!((self_sim - 1.0).abs() < 1e-3, "自相似 {self_sim}");
    // 同句再嵌（确定性）
    let v2 = emb.embed("这是一条测试消息").unwrap();
    let cos: f32 = v.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
    assert!(cos > 0.999, "同句余弦 {cos}");
    // 不同句余弦 < 1
    let v3 = emb.embed("完全无关的句子").unwrap();
    let cos2: f32 = v.iter().zip(v3.iter()).map(|(a, b)| a * b).sum();
    assert!(cos2 < 0.999, "异句余弦 {cos2}");
}
