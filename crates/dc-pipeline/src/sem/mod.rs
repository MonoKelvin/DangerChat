//! 语义判定模块（设计文档 §5.7，ADR-14 线性头方案）。
//! `Stage<OcrResult, Verdict>`。
//!
//! 判定顺序：
//! 1. **L1 规则**（永远启用，毫秒级）——命中即短路，L2 不再跑；
//! 2. **L2 嵌入 + 线性头**（可开关）——`score = sigmoid(w·embed + b)` 越阈值告警；
//!    头文件缺失 → 模板相似度兜底（**只提示不拦截**）；
//!    Embedder 加载失败 → `l2_enabled` 视为 false，仅 L1（fail-open，NFR-07）；
//! 3. 空草稿 → Safe。
//!
//! L2 特征（必须与语料 `resources/training_set/conversation/schema.json` 的
//! `text_contract` / `object_contract` 逐字一致）：
//! - 草稿塔 [`draft_embed_text`]：`之前的对话：{ctx}\n现在要发送：{draft}`；
//! - 对象塔 [`object_embed_text`]：`对象：{target}`，**仅当 head 权重为 1024 维
//!   （= 2×[`embedder::EMBED_DIM`]）时参与拼接**，否则只用草稿塔（512 维，旧头兼容）。

pub mod contacts;
pub mod doc;
pub mod embedder;
pub mod head;
pub mod rules;
pub mod scenarios;

use std::sync::RwLock;
use std::time::Instant;

use dc_core::{
    ConfigField, ConfigType, ConfigValue, MetricsRecorder, ModuleContext, ModuleError,
    ModuleMetrics,
};

use crate::contract::{Module, OcrResult, PipelineContext, Stage, StageError};
use crate::verdict::Verdict;
use contacts::ContactBook;
use rules::{Profile, RuleSet};
use scenarios::ScenarioManager;

/// 阈值（§5.7）。2026-09-18 由 tools/training/train_head.py 在 **1626 条**标注语料
/// （四段特征：草稿 ⊕ 对象 ⊕ 逐元素积 ⊕ 差）上重校准：
/// - formal 0.55 → 准确率 91.8%（扫描最优 0.51 / 92.2%）；
/// - casual 0.45 → 准确率 85.8%（扫描最优 0.47 / 86.0%）。
///
/// 最优值与现行默认差 <0.5pt（不显著），**保留既有无符号默认**以免配置与文档漂移。
///
/// ⚠️ **不再声称「零误报」**（旧注释口径来自 ~300 条语料，已失效）：现语料刻意含大量
/// 硬负例（吐槽群脏话、死党互损、尴尬但不危险），阈值处误报率 formal 8.8% / casual 13.9%，
/// 对真实流量的外推偏悲观；零误报需阈值 ≥0.84 / 0.91，此时召回仅 55% / 35%，不可用。
/// 因语料为对抗性样本，此处以「准确率最优」定阈值，硬拦截的保守性由 [`BLOCK_MARGIN`] 承担。
pub const THRESHOLD_FORMAL: f32 = 0.55;
pub const THRESHOLD_CASUAL: f32 = 0.45;
/// 升 Block 的边距：定义在 [`crate::verdict::BLOCK_MARGIN`]（实际生效处），
/// 此处重导出以保留既有引用路径。
pub use crate::verdict::BLOCK_MARGIN;

/// 模板兜底降级（ADR-14）：无训练头时 L2「只提示不拦截」，
/// 把 Block 封顶到 Warn（Safe/Warn 原样）。有训练头则不动。
fn cap_fallback_level(mut v: Verdict, has_weights: bool) -> Verdict {
    if !has_weights && v.level == crate::verdict::VerdictLevel::Block {
        v.level = crate::verdict::VerdictLevel::Warn;
    }
    v
}

/// 对象未知时的占位。固定占位而非省略，**保持特征位置稳定**
/// ——线性头是按位置的加权和，输入缺位会让同一语义落到不同位置。
const UNKNOWN_OBJECT: &str = "未标记";

/// 按 head 维度装配 L2 特征（纯函数，可脱离模型单测）。
///
/// - `EMBED_DIM`(512)：`draft`；
/// - `2×EMBED_DIM`(1024)：`draft ⊕ object`——**可分离**，对象只贡献常数偏移；
/// - `4×EMBED_DIM`(2048)：再叠加 `draft⊙object` 与 `draft−object`（**交互项**）。
///
/// 拼接顺序必须与训练脚本 `tools/training/train_head.py` 一致，改动即需重训头文件。
pub(crate) fn assemble_features(draft: &[f32], object: &[f32], dim: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(dim.max(2 * draft.len()));
    v.extend_from_slice(draft);
    if dim == draft.len() {
        return v;
    }
    v.extend_from_slice(object);
    if dim == 4 * draft.len() {
        v.extend(draft.iter().zip(object.iter()).map(|(a, b)| a * b));
        v.extend(draft.iter().zip(object.iter()).map(|(a, b)| a - b));
    }
    v
}

/// 草稿塔的嵌入输入（与 `schema.json::text_contract` 一致）。
///
/// 上下文可选：`ctx` 为空/空白视为无上下文。
pub fn draft_embed_text(ctx: Option<&str>, draft: &str) -> String {
    match ctx.map(str::trim) {
        Some(c) if !c.is_empty() => format!("之前的对话：{c}\n现在要发送：{draft}"),
        _ => draft.to_string(),
    }
}

/// 对象塔的嵌入输入（与 `schema.json::object_contract` 一致）。
///
/// **为什么需要对象塔**：仅凭 `ctx + draft` 无法区分「同一句话发给不同的人」——
/// 「宝宝你这么好看」发给恋人是正常亲密，发给死党/老师/父母则是明确错位；
/// 「今晚来我家吧，就我们俩」在无对象信息时同样不可判。
///
/// **为什么不把对象拼进草稿串**（曾实现过，已回退）：实测（2026-09-18，1545 条语料）
/// 拼入前缀会因 mean-pooling 稀释而全面变差——formal 准确率 86.9%→86.2%、
/// casual 82.9%→81.6%，误报/漏报双双上升；改为「双塔拼接」后反而提升到
/// **89.8% / 85.2%**（formal 误报 44→34、漏报 60→47）。因此对象必须作为
/// **独立特征塔**引入，而不是加长同一个串。
///
/// 已知局限：对象名来自 OCR 的会话标题，可能读取失败（→ `未标记`）。
///
/// **交互项（2048 维头）**：在拼接之上再加 `draft⊙object` 与 `draft−object`。
/// 原因：线性头在纯拼接上是**可分离**的（`score = w_d·draft + w_o·object + b`），
/// 对象只能贡献与草稿无关的常数偏移，**无法表达「同一句话 × 不同对象 → 相反判定」**
/// ——实测纯拼接下探针（「宝宝你这么好看」对女朋友 vs 对好兄弟）差 ≈0，且排序错误。
/// 换成含交互项的 2048 维后（1626 条语料，其中 85 条配对样本）：组内配对方向正确率
/// casual 82%→94%，整体准确率 formal 90.2%→91.8% / casual 83.9%→85.8%。
pub fn object_embed_text(target: Option<&str>) -> String {
    let target = match target.map(str::trim) {
        Some(t) if !t.is_empty() => t,
        _ => UNKNOWN_OBJECT,
    };
    format!("对象：{target}")
}

pub struct SemStage {
    rules: RwLock<RuleSet>,
    contacts: RwLock<ContactBook>,
    /// 场景表（id/名称/基线；L1 过滤键与 L2 基线折算的唯一来源）
    scenarios: RwLock<ScenarioManager>,
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
            rules: RwLock::new(RuleSet::from_defs(Vec::new()).unwrap_or_else(|_| {
                // from_defs 对空集永远 Ok；此处 unreachable 仅防御
                panic!("空规则集构造失败")
            })),
            contacts: RwLock::new(ContactBook::default()),
            scenarios: RwLock::new(ScenarioManager::builtin_only()),
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
            rules: RwLock::new(rules),
            contacts: RwLock::new(contacts),
            scenarios: RwLock::new(ScenarioManager::builtin_only()),
            embedder: None,
            heads: head::Heads::fallback_only(),
            l2_enabled: false,
            threshold_formal: THRESHOLD_FORMAL,
            threshold_casual: THRESHOLD_CASUAL,
            metrics: MetricsRecorder::default(),
        }
    }

    /// 热重载规则/联系人/场景（保存后由前端触发）。
    pub fn reload_rules_and_contacts(
        &self,
        rules_path: &str,
        contacts_path: &str,
        scenes_path: &str,
    ) {
        // 重载规则
        match std::fs::read_to_string(rules_path) {
            Ok(text) => match RuleSet::from_json(&text) {
                Ok(rs) => {
                    if let Ok(mut w) = self.rules.write() {
                        *w = rs;
                        tracing::info!(path = %rules_path, "规则库热重载成功");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "规则库解析失败，保持原有规则"),
            },
            Err(e) => tracing::warn!(path = %rules_path, error = %e, "规则库读取失败"),
        }

        // 重载联系人
        if let Ok(text) = std::fs::read_to_string(contacts_path) {
            match ContactBook::from_json(&text) {
                Ok(cb) => {
                    if let Ok(mut w) = self.contacts.write() {
                        *w = cb;
                        tracing::info!(path = %contacts_path, "联系人画像热重载成功");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "联系人画像解析失败，保持原有画像"),
            }
        }

        // 重载场景（文件即真相；缺失/非法由 load 内部兜底为仅内置场景）
        if let Ok(mut w) = self.scenarios.write() {
            *w = ScenarioManager::load(std::path::Path::new(scenes_path));
            tracing::info!(path = %scenes_path, "场景表热重载");
        }
    }

    /// 融合判定（§5.7 判定算法；纯逻辑，可脱离 Stage 单测）。
    ///
    /// 上下文感知（§5.7-context）：
    /// - L1 规则仅对 draft_text 判定（毫秒级，短路）；
    /// - L2 嵌入时，若 `chat_context` 存在，则将其与 draft_text 拼接为
    ///   `"之前的对话：{context}\n现在要发送：{draft}"` 再嵌入，
    ///   让线性头在对话语境下判定危险概率。
    pub fn judge(&self, input: &OcrResult) -> Verdict {
        let draft = input.draft_text.trim();

        // 读取联系人画像 → 场景 id；经场景表折算展示名与 L2 基线
        // （RwLock 中毒 → 空串 → 场景表按未知 id 兜底为 formal）
        let scenario_id = self
            .contacts
            .read()
            .map(|c| {
                c.profile_of(input.chat_target.as_deref().unwrap_or(""))
                    .to_string()
            })
            .unwrap_or_default();
        let (scenario_name, base) = match self.scenarios.read() {
            Ok(sm) => (sm.name_of(&scenario_id), sm.base_profile(&scenario_id)),
            Err(_) => (scenario_id.clone(), Profile::Formal),
        };
        // L1（永远启用，毫秒级；命中短路）
        if let Ok(rules) = self.rules.read() {
            if let Some(hit) = rules.first_hit(draft, &scenario_id) {
                return Verdict::from_rule(&hit.pattern);
            }
        }

        // 空草稫 → Safe
        if draft.is_empty() {
            return Verdict::safe();
        }

        // L2（可开关；Embedder 缺失 = fail-open 仅 L1）
        if self.l2_enabled && self.embedder.is_some() {
            // 上下文与对象各成一个特征塔（对象塔仅在 1024 维头下参与拼接，
            // 详见 draft_embed_text / object_embed_text 的文档注释与实测对比）。
            let draft_text = draft_embed_text(input.chat_context.as_deref(), draft);
            let object_text = object_embed_text(input.chat_target.as_deref());
            if let Some(score) = self.l2_score(&draft_text, &object_text, base) {
                let threshold = match base {
                    Profile::Formal => self.threshold_formal,
                    Profile::Casual => self.threshold_casual,
                };
                // 模板兜底模式（无训练头）只提示不拦截（ADR-14 降级语义）：
                // 级别封顶 Warn，绝不 Block。有训练头时正常 Warn/Block。
                let has_weights = self.heads.has_weights(base);
                let mut v = cap_fallback_level(Verdict::from_score(score, threshold), has_weights);
                if v.level != crate::verdict::VerdictLevel::Safe {
                    // has_weights 只接受 Profile（双头按基线复用，不随场景数增长）
                    let mode = if has_weights {
                        "危险分"
                    } else {
                        "兜底相似度"
                    };
                    v.reasons.push(format!(
                        "与「{scenario_name}」场景语义不匹配，{mode} {score:.2} ≥ {threshold:.2}"
                    ));
                }
                return v;
            }
        }

        Verdict::safe()
    }

    /// L2 打分。头缺失/推理失败 → None（调用方回 Safe；fail-open）。
    ///
    /// 特征按 head 维度装配（见 [`assemble_features`]）；维度不符由 head 侧守卫拒绝。
    /// 旧 head 文件（512/1024）无需重训即可继续工作，重训产出 2048 维头才启用交互项。
    fn l2_score(&self, draft_text: &str, object_text: &str, base: Profile) -> Option<f32> {
        let embedder = self.embedder.as_ref()?;
        let draft = embedder.embed(draft_text).ok()?;
        let dim = self.heads.feature_dim(base);
        if dim == embedder::EMBED_DIM {
            return self.heads.score(&draft, base);
        }
        let object = embedder.embed(object_text).ok()?;
        self.heads
            .score(&assemble_features(&draft, &object, dim), base)
    }

    fn run(&self, input: OcrResult, ctx: &PipelineContext) -> Result<Verdict, StageError> {
        ctx.cancel.check()?;
        let verdict = self.judge(&input);
        Ok(verdict
            .with_draft(
                input.draft_text.clone(),
                crate::verdict::draft_fingerprint(&input.draft_text),
            )
            .with_target(input.chat_target.clone().unwrap_or_default())
            .with_context(input.chat_context.clone()))
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
                label: "正式基线阈值".into(),
                help: "正式及基于正式基线的自定义场景使用；危险分超过该值告警，+0.15 内为警告，再高为阻断".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
            ConfigField {
                key: "sem.threshold.casual".into(),
                ty: ConfigType::Float { min: 0.1, max: 0.9 },
                default: ConfigValue::Float(THRESHOLD_CASUAL as f64),
                label: "个人基线阈值".into(),
                help: "同上，个人及基于个人基线的自定义场景使用".into(),
                group: "拦截与提示".into(),
                owner: "sem".into(),
            },
        ]
    }

    fn init(&mut self, mctx: &ModuleContext) -> Result<(), ModuleError> {
        // rules / contacts 固定在数据目录根（bootstrap 创建/bridge 保存均写此路径）。
        // 曾有 sem.rules_path 相对路径默认值导致读写错位，已移除该配置项。
        let rules_path = mctx
            .log_dir
            .parent()
            .map(|d| d.join(dc_core::paths::RULES_JSON))
            .unwrap_or_else(|| std::path::PathBuf::from(dc_core::paths::RULES_JSON));
        match std::fs::read_to_string(&rules_path) {
            Ok(text) => match RuleSet::from_json(&text) {
                Ok(rs) => {
                    if let Ok(mut w) = self.rules.write() {
                        *w = rs;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "规则库非法，按空规则集继续");
                    if let Ok(mut w) = self.rules.write() {
                        *w = RuleSet::from_defs(Vec::new()).map_err(ModuleError::Fatal)?;
                    }
                }
            },
            Err(e) => {
                tracing::warn!(path = %rules_path.display(), error = %e, "规则库不存在，按空规则集继续");
                if let Ok(mut w) = self.rules.write() {
                    *w = RuleSet::from_defs(Vec::new()).map_err(ModuleError::Fatal)?;
                }
            }
        }
        let contacts_path = mctx
            .log_dir
            .parent()
            .map(|d| d.join(dc_core::paths::CONTACTS_JSON))
            .unwrap_or_else(|| std::path::PathBuf::from(dc_core::paths::CONTACTS_JSON));
        match std::fs::read_to_string(&contacts_path) {
            Ok(text) => match ContactBook::from_json(&text) {
                Ok(cb) => {
                    if let Ok(mut w) = self.contacts.write() {
                        *w = cb;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "画像非法，全部按 formal 保守处理");
                    if let Ok(mut w) = self.contacts.write() {
                        *w = ContactBook::default();
                    }
                }
            },
            Err(_) => {
                if let Ok(mut w) = self.contacts.write() {
                    *w = ContactBook::default();
                }
            }
        }

        // 场景表（数据目录根 scenes.json；缺失 = 仅内置场景）
        let scenes_path = mctx
            .log_dir
            .parent()
            .map(|d| d.join(dc_core::paths::SCENES_JSON))
            .unwrap_or_else(|| std::path::PathBuf::from(dc_core::paths::SCENES_JSON));
        if let Ok(mut w) = self.scenarios.write() {
            *w = ScenarioManager::load(&scenes_path);
        }

        // L2：模型 + 头
        self.l2_enabled = mctx.config.bool_or("sem.l2_enabled", true);
        self.threshold_formal =
            mctx.config
                .f64_or("sem.threshold.formal", THRESHOLD_FORMAL as f64) as f32;
        self.threshold_casual =
            mctx.config
                .f64_or("sem.threshold.casual", THRESHOLD_CASUAL as f64) as f32;

        if self.l2_enabled {
            // bge 是内置模型（随安装包 resources/models/）：按名解析，
            // 用户层存在优先（允许用户覆盖），否则用内置层。
            let result = mctx
                .models
                .resolve_dir("bge")
                .ok_or_else(|| "bge 模型目录不存在（内置层与用户层均未找到）".to_string())
                .and_then(|dir| embedder::Embedder::load(&dir).map(|emb| (dir, emb)));
            match result {
                Ok((dir, emb)) => {
                    self.heads = head::Heads::load(&dir);
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

    fn process(
        &self,
        input: Self::Input,
        ctx: &PipelineContext,
    ) -> Result<Self::Output, StageError> {
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
            chat_context: None,
        }
    }

    fn stage() -> SemStage {
        let rules = RuleSet::from_defs(vec![rules::RuleDef {
            pattern: "sb".into(),
            r#match: rules::MatchKind::Word,
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
        assert_eq!(
            s.judge(&ocr("", None)).level,
            crate::verdict::VerdictLevel::Safe
        );
        assert_eq!(
            s.judge(&ocr("   ", None)).level,
            crate::verdict::VerdictLevel::Safe
        );
    }

    /// UT-SEM-05：模型未加载（fail-open）→ 仅 L1，无 panic。
    #[test]
    fn no_embedder_l1_only() {
        let s = stage();
        // L2 关闭路径：非规则文本 → Safe（不因模型缺失而 Err/panic）
        assert_eq!(
            s.judge(&ocr("普通工作消息", None)).level,
            crate::verdict::VerdictLevel::Safe
        );
    }

    /// 阈值边界（UT-SEM-04 的纯函数半边；带 L2 的在 tests/sem.rs）。
    #[test]
    fn from_score_boundaries() {
        use crate::verdict::VerdictLevel;
        let m = BLOCK_MARGIN;
        // 恰好阈值 → Warn（≥语义）
        assert_eq!(Verdict::from_score(0.55, 0.55).level, VerdictLevel::Warn);
        // 阈值 + 边距 → Block
        assert_eq!(
            Verdict::from_score(0.55 + m, 0.55).level,
            VerdictLevel::Block
        );
        // 阈值 + 边距 - 0.001 → 仍 Warn
        assert_eq!(
            Verdict::from_score(0.55 + m - 0.001, 0.55).level,
            VerdictLevel::Warn
        );
        // 阈值下 → Safe
        assert_eq!(Verdict::from_score(0.549, 0.55).level, VerdictLevel::Safe);
    }

    /// 模板兜底封顶：无训练头时 Block 降 Warn，有训练头不动（ADR-14）。
    #[test]
    fn fallback_level_capped_to_warn() {
        use crate::verdict::VerdictLevel;
        // 无训练头（模板兜底）：Block → Warn；Warn/Safe 原样
        let block = Verdict::from_score(0.80, 0.55); // Block
        assert_eq!(block.level, VerdictLevel::Block);
        assert_eq!(cap_fallback_level(block, false).level, VerdictLevel::Warn);
        let warn = Verdict::from_score(0.60, 0.55); // Warn
        assert_eq!(cap_fallback_level(warn, false).level, VerdictLevel::Warn);
        let safe = Verdict::from_score(0.10, 0.55); // Safe
        assert_eq!(cap_fallback_level(safe, false).level, VerdictLevel::Safe);
        // 有训练头：Block 保持 Block（可拦截）
        let block2 = Verdict::from_score(0.80, 0.55);
        assert_eq!(cap_fallback_level(block2, true).level, VerdictLevel::Block);
    }

    /// UT-SEM-13：L2 特征串契约（草稿塔 + 对象塔）。
    /// 该串与训练语料共享，**改动即需重训 head 文件**。
    #[test]
    fn embed_text_contract() {
        // 草稿塔：有上下文 / 无上下文（None、空串、全空白等价）
        assert_eq!(
            draft_embed_text(Some("作业做完了么"), "bro明天去哪玩呢"),
            "之前的对话：作业做完了么\n现在要发送：bro明天去哪玩呢"
        );
        assert_eq!(draft_embed_text(None, "卧槽，什么鬼"), "卧槽，什么鬼");
        assert_eq!(draft_embed_text(Some(""), "卧槽，什么鬼"), "卧槽，什么鬼");
        assert_eq!(
            draft_embed_text(Some("   "), "卧槽，什么鬼"),
            "卧槽，什么鬼"
        );
        // 对象塔：对象名两端空白被裁掉；未知 → 固定占位（不省略）
        assert_eq!(object_embed_text(Some("好兄弟")), "对象：好兄弟");
        assert_eq!(
            object_embed_text(Some("  三年二班家长群  ")),
            "对象：三年二班家长群"
        );
        assert_eq!(object_embed_text(None), "对象：未标记");
        assert_eq!(object_embed_text(Some("  ")), "对象：未标记");
    }

    /// UT-SEM-14：L2 特征装配（512 / 1024 / 2048，含交互项）。
    /// 拼接顺序与 train_head.py 的一致性由本测试锁定。
    #[test]
    fn assemble_features_dims() {
        let d = [1.0f32, 2.0, 3.0];
        let o = [0.5f32, -1.0, 2.0];
        // 仅草稿塔
        assert_eq!(assemble_features(&d, &o, 3), vec![1.0, 2.0, 3.0]);
        // 草稿塔 ⊕ 对象塔
        assert_eq!(
            assemble_features(&d, &o, 6),
            vec![1.0, 2.0, 3.0, 0.5, -1.0, 2.0]
        );
        // 再叠加 逐元素积 ⊕ 差
        assert_eq!(
            assemble_features(&d, &o, 12),
            vec![
                1.0, 2.0, 3.0, // draft
                0.5, -1.0, 2.0, // object
                0.5, -2.0, 6.0, // draft ⊙ object
                0.5, 3.0, 1.0 // draft - object
            ]
        );
    }
}
