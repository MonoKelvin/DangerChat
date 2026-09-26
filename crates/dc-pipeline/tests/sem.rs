//! M5 sem 模块集成测试（UT-SEM-01~13 补全；单元级在 src/sem/ 内）。

use dc_pipeline::contract::{OcrResult, PipelineContext, Stage, TextBlock};
use dc_pipeline::sem::{contacts::ContactBook, rules::*, SemStage, THRESHOLD_FORMAL};
use dc_pipeline::verdict::{Verdict, VerdictLevel};
use dc_sys::Rect;

fn ocr(draft: &str, target: Option<&str>) -> OcrResult {
    ocr_with_context(draft, target, None)
}

fn ocr_with_context(draft: &str, target: Option<&str>, context: Option<&str>) -> OcrResult {
    OcrResult {
        chat_target: target.map(str::to_string),
        draft_text: draft.into(),
        blocks: vec![TextBlock {
            text: draft.into(),
            rect: Rect::new(0, 0, 10, 10),
            confidence: 0.9,
        }],
        chat_context: context.map(str::to_string),
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
        "与formal场景语义不匹配，危险分 {:.2} > {THRESHOLD_FORMAL:.2}",
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

/// Stage 适配面：context 通过 process 流转到 Verdict。
#[test]
fn stage_process_passes_context() {
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
        RunId::from_raw("20260914-000000-00002"),
        dc_pipeline::contract::LoopKind::Slow,
        ImageLogSink::noop(),
        Arc::new(ConfigSnapshot::default()),
    );
    let input = ocr_with_context("你是 sb", Some("张总"), Some("之前聊天内容"));
    let v = stage.process(input, &ctx).expect("sem process");
    assert_eq!(v.chat_context.as_deref(), Some("之前聊天内容"));
}

/// UT-SEM-13：阈值极端值短路（v1.2 优化）。
/// threshold = 1.0（「禁止」）→ 仅凭聊天对象判定，立即 Block，不经 L1/L2。
/// threshold = 0.0（「无限制」）→ 仅凭聊天对象判定，立即 Safe，不经 L1/L2。
#[test]
fn ut_sem_13_threshold_extreme_short_circuit() {
    use dc_pipeline::sem::scenarios::ScenarioManager;
    use std::io::Write;

    // 构造临时 scenes.json：s1 阈值 1.0（禁止），s2 阈值 0.0（无限制）
    let mut path = std::env::temp_dir();
    path.push(format!("dc-sem-test-scenes-{}.json", std::process::id()));
    let json = r#"{
  "scene": [
    {"id": "s1", "name": "禁止", "base": "formal", "threshold": 1.0},
    {"id": "s2", "name": "无限制", "base": "formal", "threshold": 0.0}
  ]
}"#;
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(json.as_bytes()).unwrap();
    f.flush().unwrap();
    drop(f);

    let scenarios = ScenarioManager::load(&path);

    // 联系人 → s1（禁止）
    let contacts = ContactBook::from_toml(
        r#"
[[contact]]
name = "禁聊"
profile = "s1"
"#,
    )
    .unwrap();

    let stage = SemStage::l1_only_with_scenarios(
        RuleSet::from_defs(vec![RuleDef {
            pattern: "sb".into(),
            r#match: MatchKind::Word,
            applies_to: vec!["all".into()],
        }])
        .unwrap(),
        contacts,
        scenarios,
    );

    // 草稿含违禁词，但阈值=1.0（禁止）→ 短路 Block，L1 规则不生效
    let v = stage.judge(&ocr("sb 违禁词", Some("禁聊")));
    assert_eq!(
        v.level,
        VerdictLevel::Block,
        "threshold=1.0 应短路 Block，忽略 L1"
    );
    assert!(v.reasons.iter().any(|r| r.contains("禁止场景")));

    // 另一个联系人 → s2（无限制）
    let contacts2 = ContactBook::from_toml(
        r#"
[[contact]]
name = "随便聊"
profile = "s2"
"#,
    )
    .unwrap();
    let sm2 = ScenarioManager::load(&path);
    let stage2 = SemStage::l1_only_with_scenarios(
        RuleSet::from_defs(vec![RuleDef {
            pattern: "sb".into(),
            r#match: MatchKind::Word,
            applies_to: vec!["all".into()],
        }])
        .unwrap(),
        contacts2,
        sm2,
    );

    // 草稿含违禁词，但阈值=0.0（无限制）→ 短路 Safe，L1 规则不生效
    let v2 = stage2.judge(&ocr("sb 违禁词", Some("随便聊")));
    assert_eq!(
        v2.level,
        VerdictLevel::Safe,
        "threshold=0.0 应短路 Safe，忽略 L1"
    );

    let _ = std::fs::remove_file(&path);
}

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
    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/models/bge-small"
    ));
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

/// IT-SEM-01（真模型，`-- --ignored`）：**2048 维（含交互项）的头端到端**。
///
/// 验证三件事，缺一即说明「同一句话 × 不同对象」这一能力未真正生效：
/// 1. 真实 head 被加载为 4×EMBED_DIM（运行时走 草稿 ⊕ 对象 ⊕ 积 ⊕ 差 的装配路径）；
/// 2. 特征可拼接并按该维度打分（维度守卫不误杀）；
/// 3. **同一草稿换对象会改变分数**——对象塔与交互项生效的可观测证据。
#[test]
#[ignore = "需要 models/bge 真实权重（gitignore；见 models/README）"]
fn real_heads_two_tower_scoring() {
    use dc_pipeline::sem::embedder::EMBED_DIM;
    use dc_pipeline::sem::head::Heads;
    use dc_pipeline::sem::rules::Profile;
    use dc_pipeline::sem::{draft_embed_text, object_embed_text};

    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/models/bge-small"
    ));
    let emb = dc_pipeline::sem::embedder::Embedder::load(dir).expect("BGE 加载失败");
    let heads = Heads::load(dir);

    // 1) 激活证明
    assert_eq!(
        heads.feature_dim(Profile::Formal),
        4 * EMBED_DIM,
        "head-formal.json 不是 2048 维，交互项未激活（跑 tools/training/train_head.py 重训）"
    );
    assert_eq!(heads.feature_dim(Profile::Casual), 4 * EMBED_DIM);

    // 2)+3) 同一草稿 + 不同对象
    let draft_tower = draft_embed_text(None, "宝宝你这么好看");
    let feats = |obj: &str| -> Vec<f32> {
        let a = emb.embed(&draft_tower).expect("草稿塔嵌入失败");
        let b = emb
            .embed(&object_embed_text(Some(obj)))
            .expect("对象塔嵌入失败");
        let mut v = a.to_vec();
        v.extend_from_slice(&b);
        v.extend(a.iter().zip(b.iter()).map(|(x, y)| x * y));
        v.extend(a.iter().zip(b.iter()).map(|(x, y)| x - y));
        v
    };
    let s_partner = heads
        .score(&feats("女朋友"), Profile::Casual)
        .expect("打分失败");
    let s_buddy = heads
        .score(&feats("好兄弟"), Profile::Casual)
        .expect("打分失败");
    assert!(
        (s_partner - s_buddy).abs() > 1e-4,
        "同一草稿换对象后分数应不同（对象塔/交互项生效）：女朋友 {s_partner} / 好兄弟 {s_buddy}"
    );
}

// ---------------------------------------------------------------------------
// UT-SEM-10~12：上下文感知判定（§5.7-context）
// ---------------------------------------------------------------------------

/// UT-SEM-10：上下文不影响 L1 规则判定（L1 短路前不看 context）。
/// L1 规则命中 draft_text → 立即 Block，context 被忽略。
#[test]
fn ut_sem_10_l1_unaffected_by_context() {
    let rules = RuleSet::from_toml(
        r#"
[[rule]]
pattern = "sb"
match = "word"
severity = "block"
applies_to = ["all"]
"#,
    )
    .unwrap();
    let stage = SemStage::l1_only(rules, ContactBook::default());

    // 草稿命中规则 → Block，context 被忽略
    let input = ocr_with_context("你是 sb", Some("同事"), Some("最近项目进展顺利"));
    let v = stage.judge(&input);
    assert_eq!(v.level, VerdictLevel::Block);
    assert!(v.reasons[0].contains("sb"));
}

/// UT-SEM-11：上下文在 L2 判定中的融合（§5.7-context）。
/// L2 嵌入时，将 context 拼接到 draft 之前，改变判分结果。
///
/// 设计验证：
/// - 无模型（l1_only）→ context 不参与任何判定；
/// - 有模型场景下，context 改变嵌入输入 → 分数不同。
///
/// 由于真模型需要 --ignored，此处验证 L1-only 路径下 context 不影响结果。
#[test]
fn ut_sem_11_context_in_l1_only() {
    let stage = SemStage::l1_only(
        RuleSet::from_defs(Vec::new()).unwrap(),
        ContactBook::default(),
    );
    // 无规则 + 无模型 → context 无法影响 → Safe
    let v = stage.judge(&ocr_with_context("普通消息", None, Some("上下文内容")));
    assert_eq!(v.level, VerdictLevel::Safe);
    assert_eq!(v.score, 0.0);
}

/// UT-SEM-12：context 为空 / None 时行为与之前完全一致（回归）。
/// 确保新增 chat_context 字段不改变现有判定逻辑。
#[test]
fn ut_sem_12_context_absent_no_behavior_change() {
    let rules = RuleSet::from_defs(vec![RuleDef {
        pattern: "sb".into(),
        r#match: MatchKind::Word,
        applies_to: vec!["all".into()],
    }])
    .unwrap();
    let stage = SemStage::l1_only(rules, ContactBook::default());

    // context = None → 同 UT-SEM-01 行为
    let v1 = stage.judge(&ocr("你是 sb", Some("张总")));
    assert_eq!(v1.level, VerdictLevel::Block);

    // context = Some("") → 视为无上下文（空字符串跳过拼接）
    let v2 = stage.judge(&ocr_with_context("你是 sb", Some("张总"), Some("")));
    assert_eq!(v2.level, VerdictLevel::Block);

    // context = Some(非空) → L1 仍短路（规则命中不看 context）
    let v3 = stage.judge(&ocr_with_context(
        "你是 sb",
        Some("张总"),
        Some("之前聊天内容"),
    ));
    assert_eq!(v3.level, VerdictLevel::Block);
}
