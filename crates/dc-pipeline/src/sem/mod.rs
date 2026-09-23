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

pub use head::{FastPath, ScoreResult};

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
/// 因语料为对抗性样本，此处以「准确率最优」定阈值；硬拦截的保守性由场景阈值下沉承担。
pub const THRESHOLD_FORMAL: f32 = 0.55;
pub const THRESHOLD_CASUAL: f32 = 0.45;
/// `BLOCK_MARGIN` 重导出，供文档参考（v1.2 判定直接用 `score > threshold`）。
pub use crate::verdict::BLOCK_MARGIN;

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
    /// L2 编码器：None = 未加载（fail-open 仅 L1）。
    /// 用 RwLock 而非裸 Option：挂起态（目标非前台）由 worker 线程 `unload_model`
    /// 释放 bge-small 的 ~50MB 会话内存，回到前台再 `ensure_loaded` 重载
    /// （§2.4 挂起卸载；bge-small 常驻内存与「内存极低」目标冲突，靠按需装卸解决）。
    embedder: RwLock<Option<embedder::Embedder>>,
    /// 已解析的模型目录（init 时确定；卸载后重载的依据）。
    model_dir: RwLock<Option<std::path::PathBuf>>,
    /// 头权重：小（每头几十 KB），常驻，不随挂起卸载。
    heads: head::Heads,
    l2_enabled: bool,
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
            embedder: RwLock::new(None),
            model_dir: RwLock::new(None),
            heads: head::Heads::fallback_only(),
            l2_enabled: false,
            metrics: MetricsRecorder::default(),
        }
    }

    /// 测试用：仅 L1。
    pub fn l1_only(rules: RuleSet, contacts: ContactBook) -> Self {
        Self::l1_only_with_scenarios(rules, contacts, ScenarioManager::builtin_only())
    }

    /// 测试用：仅 L1，带自定义场景管理器。
    pub fn l1_only_with_scenarios(
        rules: RuleSet,
        contacts: ContactBook,
        scenarios: ScenarioManager,
    ) -> Self {
        Self {
            rules: RwLock::new(rules),
            contacts: RwLock::new(contacts),
            scenarios: RwLock::new(scenarios),
            embedder: RwLock::new(None),
            model_dir: RwLock::new(None),
            heads: head::Heads::fallback_only(),
            l2_enabled: false,
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
        let (scenario_name, base, threshold) = match self.scenarios.read() {
            Ok(sm) => (
                sm.name_of(&scenario_id),
                sm.base_of(&scenario_id),
                sm.threshold_of(&scenario_id),
            ),
            Err(_) => (scenario_id.clone(), Profile::Formal, THRESHOLD_FORMAL),
        };

        // 阈值极端值短路（v1.2 优化）：
        // - threshold = 1.0（「禁止」）：仅凭聊天对象判断为该场景 → 立即 Block，不经 L1/L2。
        // - threshold = 0.0（「无限制」）：仅凭聊天对象判断为该场景 → 立即 Safe，不经 L1/L2。
        // 两种场景均跳过 OCR 文字识别与 L2 推理，仅依据聊天对象所属场景判定。
        if (threshold - 1.0).abs() < 1e-6 {
            return Verdict::block("禁止场景").with_scene(scenario_name);
        }
        if threshold.abs() < 1e-6 {
            return Verdict::safe().with_scene(scenario_name);
        }

        // L1（永远启用，毫秒级；命中短路）
        if let Ok(rules) = self.rules.read() {
            if let Some(hit) = rules.first_hit(draft, &scenario_id) {
                return Verdict::from_rule(&hit.pattern).with_scene(scenario_name);
            }
        }

        // 空草稫 → Safe
        if draft.is_empty() {
            return Verdict::safe().with_scene(scenario_name);
        }

        // L2（可开关；Embedder 缺失/已卸载 = fail-open 仅 L1）
        let embedder_loaded = self
            .embedder
            .read()
            .map(|e| e.is_some())
            .unwrap_or(false);
        if self.l2_enabled && embedder_loaded {
            // 上下文与对象各成一个特征塔（对象塔仅在 1024 维头下参与拼接，
            // 详见 draft_embed_text / object_embed_text 的文档注释与实测对比）。
            let draft_text = draft_embed_text(input.chat_context.as_deref(), draft);
            let object_text = object_embed_text(input.chat_target.as_deref());
            if let Some(draft) = self.l2_draft(&draft_text) {
                // fast-path： draft-only 得分落在 safe_band/block_band → 短路判定。
                match self.heads.judge_fast(&draft, base) {
                    ScoreResult::ShortCircuitSafe => {
                        return Verdict::safe().with_scene(scenario_name);
                    }
                    ScoreResult::ShortCircuitBlock => {
                        return Verdict::block("语义 fast-path 短路").with_scene(scenario_name);
                    }
                    ScoreResult::FullScore { .. } => {}
                }
                // fast-path 未命中 → 完整双塔评分
                let dim = self.heads.feature_dim(base);
                if dim == embedder::EMBED_DIM {
                    if let Some(score) = self.heads.score(&draft, base) {
                        let has_weights = self.heads.has_weights(base);
                        let mut v = if has_weights {
                            Verdict::from_score(score, threshold)
                        } else {
                            Verdict::safe()
                        };
                        if v.level == crate::verdict::VerdictLevel::Block {
                            v.reasons.push(format!(
                                "场景语义不匹配，危险分 {score:.2} > {threshold:.2}"
                            ));
                        }
                        return v.with_scene(scenario_name);
                    }
                } else {
                    if let Some(score) = self.l2_score_obj(&draft, &object_text, base) {
                        let has_weights = self.heads.has_weights(base);
                        let mut v = if has_weights {
                            Verdict::from_score(score, threshold)
                        } else {
                            Verdict::safe()
                        };
                        if v.level == crate::verdict::VerdictLevel::Block {
                            v.reasons.push(format!(
                                "场景语义不匹配，危险分 {score:.2} > {threshold:.2}"
                            ));
                        }
                        return v.with_scene(scenario_name);
                    }
                }
            }
        }

        Verdict::safe().with_scene(scenario_name)
    }

    /// draft 塔嵌入：[CLS] [unused1] {draft_text} → 位置 1 隐藏态（L2 归一化）。
    fn l2_draft(&self, draft_text: &str) -> Option<[f32; embedder::EMBED_DIM]> {
        let guard = self.embedder.read().ok()?;
        let embedder = guard.as_ref()?;
        embedder.embed_draft(draft_text).ok()
    }

    /// 完整双塔评分（head 维度决定是否拼接对象塔）。
    fn l2_score_obj(&self, draft: &[f32], object_text: &str, base: Profile) -> Option<f32> {
        let dim = self.heads.feature_dim(base);
        if dim == embedder::EMBED_DIM {
            return self.heads.score(draft, base);
        }
        let guard = self.embedder.read().ok()?;
        let embedder = guard.as_ref()?;
        // 对象塔用 [unused2] 标记：[CLS] [unused2] {object_text}
        let object = embedder.embed_object(object_text).ok()?;
        self.heads.score(&assemble_features(draft, &object, dim), base)
    }

    /// 挂起态卸载（§2.4）：目标程序离开前台时释放 bge-small 会话内存（~50MB）。
    /// heads（小）与配置保留，回到前台经 [`ensure_loaded`] 重载（~100ms）。
    /// 幂等：已卸载再调无副作用。仅在 l2_enabled 且已加载过时打印日志。
    pub fn unload_model(&self) {
        if let Ok(mut w) = self.embedder.write() {
            if w.take().is_some() {
                tracing::info!("sem 挂起：已卸载语义模型，释放会话内存");
            }
        }
    }

    /// 回前台重载（§2.4）：挂起卸载后目标重新前台时调用。
    /// 幂等：已加载直接返回。重载失败 → 保持 None（fail-open 仅 L1）。
    pub fn ensure_loaded(&self) {
        if !self.l2_enabled {
            return;
        }
        if self.embedder.read().map(|e| e.is_some()).unwrap_or(true) {
            return; // 已加载，或锁中毒（不冒进重载）
        }
        let dir = match self.model_dir.read().ok().and_then(|d| d.clone()) {
            Some(d) => d,
            None => return,
        };
        match embedder::Embedder::load(&dir) {
            Ok(emb) => {
                if let Ok(mut w) = self.embedder.write() {
                    *w = Some(emb);
                    tracing::info!("sem 唤醒：语义模型已重载");
                }
            }
            Err(e) => tracing::warn!(error = %e, "sem 唤醒重载失败，本轮仅 L1（fail-open）"),
        }
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
                key: "sem.model".into(),
                ty: ConfigType::Text { max_len: 64 },
                default: ConfigValue::Str("bge-small".into()),
                label: "语义模型".into(),
                help: "models/ 下 kind=sem 的模型目录名；内置 bge-small，用户可放入自训练/替换模型后在此切换".into(),
                group: "模型与设备".into(),
                owner: "sem".into(),
            },
            ConfigField {
                key: "sem.l2_enabled".into(),
                ty: ConfigType::Bool,
                default: ConfigValue::Bool(true),
                label: "语义模型判定".into(),
                help: "L2 嵌入判定；关闭后仅规则引擎".into(),
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
        // 文件缺失/非法时回落**内置基线词库**（而非空集）——保证明显脏话开箱即拦。
        match std::fs::read_to_string(&rules_path) {
            Ok(text) => match RuleSet::from_json(&text) {
                Ok(rs) => {
                    if let Ok(mut w) = self.rules.write() {
                        *w = rs;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "规则库非法，回落内置基线词库");
                    if let Ok(mut w) = self.rules.write() {
                        *w = RuleSet::from_defs(rules::baseline_defs()).map_err(ModuleError::Fatal)?;
                    }
                }
            },
            Err(e) => {
                tracing::warn!(path = %rules_path.display(), error = %e, "规则库不存在，回落内置基线词库");
                if let Ok(mut w) = self.rules.write() {
                    *w = RuleSet::from_defs(rules::baseline_defs()).map_err(ModuleError::Fatal)?;
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

        if self.l2_enabled {
            // 语义模型可切换（sem.model 配置项，默认内置 bge-small）：
            // 用户把自训练/替换模型（kind=sem）放进数据目录 models/ 下即可在设置里选。
            // 用户层同名覆盖内置层（resolve_dir 语义）。
            //
            // **惰性加载（内存优化）**：init 只解析目录与头权重（几十 KB），
            // **不加载 ~50MB 的 Embedder 会话**——真正的加载延后到 `ensure_loaded`，
            // 由 worker 在「目标程序前台」时触发。这样目标不在前台/用户不打字时，
            // 常驻内存只有头权重，实测空闲 RSS 从 ~60MB 降到 ~30-40MB。
            let model_name = mctx.config.str_or("sem.model", "bge-small");
            // kind 校验：非 sem 模型直接拒绝（避免选错把 layout/ocr 模型当语义模型加载）。
            let kind_ok = mctx
                .models
                .get(model_name)
                .map(|info| info.kind() == Some(dc_core::ModelKind::Sem))
                .unwrap_or(false);
            match (kind_ok, mctx.models.resolve_dir(model_name)) {
                (true, Some(dir)) => {
                    // heads::load 惰性创建 embedder：线性头不落模板兜底分支 → 不加载会话，仅读头权重。
                    self.heads = head::Heads::load(&dir);
                    if let Ok(mut w) = self.model_dir.write() {
                        *w = Some(dir); // 记录目录：ensure_loaded 据此按需加载
                    }
                    tracing::info!(
                        model = %model_name,
                        heads = self.heads.describe(),
                        "sem L2 头已就绪（Embedder 惰性加载，前台时才装入）"
                    );
                }
                _ => {
                    // fail-open（NFR-07）：模型缺失或 kind 不符 → 仅 L1
                    tracing::warn!(
                        model = %model_name,
                        "语义模型不存在或 kind 非 sem，仅启用 L1 规则判定（可在设置中放入模型并切换）"
                    );
                    self.l2_enabled = false;
                }
            }
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), ModuleError> {
        if let Ok(mut w) = self.embedder.write() {
            *w = None;
        }
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
        // score > threshold → Block
        assert_eq!(Verdict::from_score(0.80, 0.55).level, VerdictLevel::Block);
        // score == threshold → Safe（> 不含等）
        assert_eq!(Verdict::from_score(0.55, 0.55).level, VerdictLevel::Safe);
        // score > threshold + ε → Block
        assert_eq!(Verdict::from_score(0.55 + 1e-3, 0.55).level, VerdictLevel::Block);
        // score < threshold → Safe
        assert_eq!(Verdict::from_score(0.549, 0.55).level, VerdictLevel::Safe);
    }

    /// UT-SEM-04 补充：移除 Warn 等级，from_score 仅返回 Block 或 Safe。
    #[test]
    fn from_score_only_block_or_safe() {
        use crate::verdict::VerdictLevel;
        // score > threshold → Block
        assert_eq!(Verdict::from_score(0.80, 0.55).level, VerdictLevel::Block);
        // score == threshold → Safe（> 不含等）
        assert_eq!(Verdict::from_score(0.55, 0.55).level, VerdictLevel::Safe);
        // score < threshold → Safe
        assert_eq!(Verdict::from_score(0.54, 0.55).level, VerdictLevel::Safe);
        // threshold=0（无限制）: judge() 短路 Safe，这里验证 from_score 数学行为
        assert_eq!(Verdict::from_score(0.01, 0.0).level, VerdictLevel::Block);
        // threshold=0（无限制）: score = 0 → Safe
        assert_eq!(Verdict::from_score(0.0, 0.0).level, VerdictLevel::Safe);
        // threshold=1（禁止）: judge() 短路 Block，这里验证 from_score 数学行为
        assert_eq!(Verdict::from_score(1.0, 1.0).level, VerdictLevel::Safe);
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
