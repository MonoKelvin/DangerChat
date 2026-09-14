//! 语义判定模块（设计文档 §5.7，ADR-14 线性头方案）。
//! `Stage<OcrResult, Verdict>`。
//!
//! 判定顺序：
//! 1. **L1 规则**（永远启用，毫秒级）——命中即短路，L2 不再跑；
//! 2. **L2 嵌入 + 线性头**（可开关）——`score = sigmoid(w·embed + b)` 越阈值告警；
//!    头文件缺失 → 模板相似度兜底（**只提示不拦截**）；
//!    Embedder 加载失败 → `l2_enabled` 视为 false，仅 L1（fail-open，NFR-07）；
//! 3. 空草稿 → Safe。

pub mod contacts;
pub mod embedder;
pub mod head;
pub mod rules;

use std::time::Instant;

use dc_core::{
    ConfigField, ConfigType, ConfigValue, MetricsRecorder, ModuleContext, ModuleError,
    ModuleMetrics,
};

use crate::contract::{Module, OcrResult, PipelineContext, Stage, StageError};
use crate::verdict::Verdict;
use contacts::ContactBook;
use rules::{Profile, RuleSet};

/// 阈值（§5.7；默认值待 ≥200 条样本校准后定稿）。
pub const THRESHOLD_FORMAL: f32 = 0.55;
pub const THRESHOLD_CASUAL: f32 = 0.35;
/// score 超过阈值该区间升 Block。
pub const BLOCK_MARGIN: f32 = 0.15;

pub struct SemStage {
    rules: RuleSet,
    contacts: ContactBook,
    /// L2 三件：None = 未加载（fail-open 仅 L1）。
    embedder: Option<embedder::Embedder>,
    heads: head::Heads,
    l2_enabled: bool,
    threshold_formal: f32,
    threshold_casual: f32,
    metrics: MetricsRecorder,
}

impl SemStage {
    /// L2 头状态描述（日志/诊断用）。
    pub fn head_state(&self) -> &'static str {
        self.heads.describe()
    }

    /// 空构造（init 前占位；guard 装配用）。
    pub fn empty() -> Self {
        Self {
            rules: RuleSet::from_defs(Vec::new()).unwrap_or_else(|_| {
                // from_defs 对空集永远 Ok；此处 unreachable 仅防御
                panic!("空规则集构造失败")
            }),
            contacts: ContactBook::default(),
            embedder: None,
            heads: head::Heads::fallback_only(),
            l2_enabled: false,
            threshold_formal: THRESHOLD_FORMAL,
            threshold_casual: THRESHOLD_CASUAL,
            metrics: MetricsRecorder::default(),
        }
    }

    /// 测试用：仅 L1。
    pub fn l1_only(rules: RuleSet, contacts: ContactBook) -> Self {
        Self {
            rules,
            contacts,
            embedder: None,
            heads: head::Heads::fallback_only(),
            l2_enabled: false,
            threshold_formal: THRESHOLD_FORMAL,
            threshold_casual: THRESHOLD_CASUAL,
            metrics: MetricsRecorder::default(),
        }
    }

    /// 融合判定（§5.7 判定算法；纯逻辑，可脱离 Stage 单测）。
    pub fn judge(&self, input: &OcrResult) -> Verdict {
        let draft = input.draft_text.trim();
        let profile = self.contacts.profile_of(input.chat_target.as_deref().unwrap_or(""));

        // L1（永远启用，毫秒级；命中短路）
        if let Some(hit) = self.rules.first_hit(draft, profile) {
            return Verdict::from_rule(&hit.pattern, hit.severity.into());
        }

        // 空草稿 → Safe
        if draft.is_empty() {
            return Verdict::safe();
        }

        // L2（可开关；Embedder 缺失 = fail-open 仅 L1）
        if self.l2_enabled && self.embedder.is_some() {
            if let Some(score) = self.l2_score(draft, profile) {
                let threshold = match profile {
                    Profile::Formal => self.threshold_formal,
                    Profile::Casual => self.threshold_casual,
                };
                let mut v = Verdict::from_score(score, threshold);
                if v.level != crate::verdict::VerdictLevel::Safe {
                    let mode = if self.heads.has_weights(profile) {
                        "危险分"
                    } else {
                        "兜底相似度"
                    };
                    v.reasons.push(format!(
                        "与{profile}场景语义不匹配，{mode} {score:.2} ≥ {threshold:.2}"
                    ));
                }
                return v;
            }
        }

        Verdict::safe()
    }

    /// L2 打分。头缺失/推理失败 → None（调用方回 Safe；fail-open）。
    fn l2_score(&self, draft: &str, profile: Profile) -> Option<f32> {
        let embedder = self.embedder.as_ref()?;
        let embed = embedder.embed(draft).ok()?;
        self.heads.score(&embed, profile)
    }

    fn run(&self, input: OcrResult, ctx: &PipelineContext) -> Result<Verdict, StageError> {
        ctx.cancel.check()?;
        let verdict = self.judge(&input);
        Ok(verdict
            .with_draft(input.draft_text.clone(), crate::verdict::draft_fingerprint(&input.draft_text))
            .with_target(input.chat_target.clone().unwrap_or_default()))
    }
}

impl Module for SemStage {
    fn id(&self) -> &'static str {
        "sem"
    }

    fn config_schema(&self) -> Vec<ConfigField> {
        vec![
            ConfigField {
                key: "sem.l2_enabled".into(),
                ty: ConfigType::Bool,
                default: ConfigValue::Bool(true),
                label: "语义模型判定".into(),
                help: "L2 嵌入判定；关闭后仅规则引擎".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
            ConfigField {
                key: "sem.threshold.formal".into(),
                ty: ConfigType::Float { min: 0.1, max: 0.9 },
                default: ConfigValue::Float(THRESHOLD_FORMAL as f64),
                label: "正式场景阈值".into(),
                help: "危险分超过该值告警，+0.15 内为警告，再高为阻断".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
            ConfigField {
                key: "sem.threshold.casual".into(),
                ty: ConfigType::Float { min: 0.1, max: 0.9 },
                default: ConfigValue::Float(THRESHOLD_CASUAL as f64),
                label: "随意场景阈值".into(),
                help: "同上，随意（好友/家人）场景".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
            ConfigField {
                key: "sem.rules_path".into(),
                ty: ConfigType::Path,
                default: ConfigValue::Str("config/rules.toml".into()),
                label: "违禁词库".into(),
                help: "rules.toml 路径（FR-SEM-04，支持热重载）".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
            ConfigField {
                key: "sem.contacts_path".into(),
                ty: ConfigType::Path,
                default: ConfigValue::Str("config/contacts.toml".into()),
                label: "聊天对象画像".into(),
                help: "contacts.toml 路径（FR-SEM-03）".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
        ]
    }

    fn init(&mut self, mctx: &ModuleContext) -> Result<(), ModuleError> {
        // rules / contacts（文件缺失 → 空规则集，仅 L2 + warn，不 Fatal）
        // 规则库默认在数据目录根（bootstrap 创建）；配置可覆盖（绝对路径）
        let rules_default = mctx
            .log_dir
            .parent()
            .map(|d| d.join("rules.toml"))
            .unwrap_or_else(|| std::path::PathBuf::from("rules.toml"));
        let rules_default_str = rules_default.to_string_lossy().into_owned();
        let rules_path = mctx
            .config
            .str_or("sem.rules_path", &rules_default_str);
        match std::fs::read_to_string(rules_path) {
            Ok(text) => match RuleSet::from_toml(&text) {
                Ok(rs) => self.rules = rs,
                Err(e) => {
                    tracing::warn!(error = %e, "规则库非法，按空规则集继续");
                    self.rules = RuleSet::from_defs(Vec::new()).map_err(ModuleError::Fatal)?;
                }
            },
            Err(e) => {
                tracing::warn!(path = %rules_path, error = %e, "规则库不存在，按空规则集继续");
                self.rules = RuleSet::from_defs(Vec::new()).map_err(ModuleError::Fatal)?;
            }
        }
        let contacts_default = mctx
            .log_dir
            .parent()
            .map(|d| d.join("contacts.toml"))
            .unwrap_or_else(|| std::path::PathBuf::from("contacts.toml"));
        let contacts_default_str = contacts_default.to_string_lossy().into_owned();
        let contacts_path = mctx
            .config
            .str_or("sem.contacts_path", &contacts_default_str);
        match std::fs::read_to_string(contacts_path) {
            Ok(text) => match ContactBook::from_toml(&text) {
                Ok(cb) => self.contacts = cb,
                Err(e) => {
                    tracing::warn!(error = %e, "画像非法，全部按 formal 保守处理");
                    self.contacts = ContactBook::default();
                }
            },
            Err(_) => self.contacts = ContactBook::default(),
        }

        // L2：模型 + 头
        self.l2_enabled = mctx.config.bool_or("sem.l2_enabled", true);
        self.threshold_formal = mctx.config.f64_or("sem.threshold.formal", THRESHOLD_FORMAL as f64) as f32;
        self.threshold_casual = mctx.config.f64_or("sem.threshold.casual", THRESHOLD_CASUAL as f64) as f32;

        if self.l2_enabled {
            let root = mctx.models.root().to_path_buf();
            match embedder::Embedder::load(&root.join("bge")) {
                Ok(emb) => {
                    self.heads = head::Heads::load(&root.join("bge"));
                    self.embedder = Some(emb);
                    tracing::info!(
                        heads = self.heads.describe(),
                        "sem L2 已加载（ADR-14 嵌入+线性头）"
                    );
                }
                Err(e) => {
                    // fail-open（NFR-07）：仅 L1
                    tracing::warn!(error = %e, "语义模型加载失败，仅启用 L1 规则判定");
                    self.l2_enabled = false;
                }
            }
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ModuleError> {
        self.embedder = None;
        Ok(())
    }

    fn metrics(&self) -> ModuleMetrics {
        self.metrics.snapshot()
    }
}

impl Stage for SemStage {
    type Input = OcrResult;
    type Output = Verdict;

    fn process(&self, input: Self::Input, ctx: &PipelineContext) -> Result<Self::Output, StageError> {
        let started = Instant::now();
        let result = self.run(input, ctx);
        self.metrics.record(started.elapsed());
        if result.is_err() {
            self.metrics.record_failure();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::TextBlock;
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

    fn stage() -> SemStage {
        let rules = RuleSet::from_defs(vec![rules::RuleDef {
            pattern: "sb".into(),
            r#match: rules::MatchKind::Word,
            severity: rules::Severity::Block,
            applies_to: vec!["formal".into()],
        }])
        .unwrap();
        let contacts = ContactBook::from_toml(
            r#"[[contact]]
name = "文件传输助手"
profile = "casual"
"#,
        )
        .unwrap();
        SemStage::l1_only(rules, contacts)
    }

    /// UT-SEM-01（部分）：L1 规则 → Verdict 级别与理由（完整数据驱动矩阵在 tests/sem.rs）。
    #[test]
    fn l1_hit_short_circuits() {
        let s = stage();
        let v = s.judge(&ocr("你是 sb", Some("张总")));
        assert_eq!(v.level, crate::verdict::VerdictLevel::Block);
        assert!(v.reasons[0].contains("sb"));
        assert!(v.reasons[0].contains("block"));
    }

    /// UT-SEM-02：未标记对象按 formal 保守处理。
    #[test]
    fn unmarked_target_is_formal() {
        let s = stage();
        // 未标记 → formal → 命中 formal 规则
        assert_eq!(
            s.judge(&ocr("你是 sb", Some("陌生人"))).level,
            crate::verdict::VerdictLevel::Block
        );
        // 标记 casual → 不命中 formal-only 规则 → Safe
        assert_eq!(
            s.judge(&ocr("你是 sb", Some("文件传输助手"))).level,
            crate::verdict::VerdictLevel::Safe
        );
    }

    /// UT-SEM-06：空草稿 → Safe。
    #[test]
    fn empty_draft_safe() {
        let s = stage();
        assert_eq!(s.judge(&ocr("", None)).level, crate::verdict::VerdictLevel::Safe);
        assert_eq!(s.judge(&ocr("   ", None)).level, crate::verdict::VerdictLevel::Safe);
    }

    /// UT-SEM-05：模型未加载（fail-open）→ 仅 L1，无 panic。
    #[test]
    fn no_embedder_l1_only() {
        let s = stage();
        // L2 关闭路径：非规则文本 → Safe（不因模型缺失而 Err/panic）
        assert_eq!(s.judge(&ocr("普通工作消息", None)).level, crate::verdict::VerdictLevel::Safe);
    }

    /// 阈值边界（UT-SEM-04 的纯函数半边；带 L2 的在 tests/sem.rs）。
    #[test]
    fn from_score_boundaries() {
        use crate::verdict::VerdictLevel;
        // 恰好阈值 → Warn（≥语义）
        assert_eq!(Verdict::from_score(0.55, 0.55).level, VerdictLevel::Warn);
        // 阈值+0.15 → Block
        assert_eq!(Verdict::from_score(0.70, 0.55).level, VerdictLevel::Block);
        // 阈值+0.149 → 仍 Warn
        assert_eq!(Verdict::from_score(0.699, 0.55).level, VerdictLevel::Warn);
        // 阈值下 → Safe
        assert_eq!(Verdict::from_score(0.549, 0.55).level, VerdictLevel::Safe);
    }
}
