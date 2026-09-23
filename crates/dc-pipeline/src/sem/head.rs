//! 场景头（§5.7 ScenarioStore，ADR-14）：预训练线性头 + 模板相似度兜底。
//!
//! 头文件 `models/bge-small/head-{formal,casual}.json`：
//! ```json
//! {
//!   "dim": 2048,
//!   "weights": [2048 个 f32],   // 可省略 → 纯模板兜底（只提示不拦截）
//!   "bias": 0.0,
//!   "templates": ["正式场景描述…", "..."],   // 兜底模式的相似度锚点
//!   "fast_path": {
//!     "safe_band": [0.0, 0.30],   // draft-only 得分落在此区间 → 直接 Safe 短路
//!     "block_band": [0.70, 1.0]   // draft-only 得分落在此区间 → 直接 Block 短路
//!   }
//! }
//! ```
//! - 有 weights：`score = sigmoid(w·embed + b)`（可拦截）；
//! - 无 weights：与模板最大余弦的补数当分数，**输出只用于提示**（由调用方
//!   `has_weights` 判定后限制级别，不 Block——ADR-14 的降级语义）。
//! - `fast_path` 为选填：draft-only 得分在 `safe_band` 或 `block_band` 范围 →
//!   短路判定，跳过 object 塔 + 交互项拼接。详见 [`HeadFile::fast_path`]。
//!
//! ### jev 多问题头（§5.7-jev）
//!
//! 头文件可选 `jev` 字段，包含 severity 和 danger 两个子头：
//! ```json
//!
//! "jev": {
//!   "severity": {"weights": [...], "bias": 0.0},
//!   "danger":  { "weights": [...], "bias": 0.0}
//! }
//! ```
//!
//! - **jev.danger** 优先：当存在 jev.danger 时，用 danger 头打分
//!   (`sigmoid(w·features + b)`)，替代 legacy `weights`。
//! - **jev.severity** 用于 fast-path：draft-only 得分用 severity 头的前
//!   EMBED_DIM 维权重（draft_w），在 safe_band/block_band 区间短路。
//! - **降级规则**：
//!   - 有 jev.danger → 用 danger 头（可拦截）；
//!   - 有 jev.severity 但无 jev.danger → severity 头打分（降级）；
//!   - 无 jev → 用 legacy `weights`；
//!   - 无 weights 且无 jev → 模板兜底（只提示不拦截）。

use serde::{Deserialize, Serialize};

use super::embedder::EMBED_DIM;
use super::rules::Profile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadFile {
    pub dim: usize,
    /// 缺省 = 纯模板兜底模式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<Vec<f32>>,
    #[serde(default)]
    pub bias: f32,
    /// 兜底相似度锚点（正式/个人场景各一段描述即可）。
    #[serde(default)]
    pub templates: Vec<String>,
    /// fast-path 配置（选填）：draft-only 得分区间短路判定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_path: Option<FastPath>,
    /// jev 多问题头（选填）：severity（用于 fast-path draft-only 评分）
    /// + danger（用于完整双塔评分，替代 legacy weights）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev: Option<JevHeads>,
}

/// fast-path 判定结果（§5.7-jev）。
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreResult {
    /// draft-only 得分落在 safe_band → 直接 Safe，跳过完整双塔。
    ShortCircuitSafe,
    /// draft-only 得分落在 block_band → 直接 Block，跳过完整双塔。
    ShortCircuitBlock,
    /// 需要完整双塔评分（draft_score 仅用于日志/阈值参考）。
    FullScore { draft_score: f32 },
}

/// jev 多问题子头（§5.7-jev）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuestionHead {
    /// 线性头权重。
    pub weights: Vec<f32>,
    /// 线性头偏置。
    pub bias: f32,
}

/// jev 多问题头集合（§5.7-jev）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JevHeads {
    /// severity 头：用于 fast-path draft-only 评分。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<QuestionHead>,
    /// danger 头：用于完整双塔评分（优先于 legacy weights）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub danger: Option<QuestionHead>,
}

/// fast-path 区间（§5.7-jev）。`None` = 关闭 fast path。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct FastPath {
    /// draft-only 得分落在此区间 → 直接 Safe 短路。
    pub safe_band: [f32; 2],
    /// draft-only 得分落在此区间 → 直接 Block 短路。
    pub block_band: [f32; 2],
}

#[derive(Debug, Clone)]
enum Head {
    /// 训练过的线性头（可拦截）。
    Linear {
        weights: Vec<f32>,
        bias: f32,
        draft_w: Vec<f32>,
        draft_b: f32,
        fast_path: Option<FastPath>,
        /// jev 多问题头（优先级高于 legacy weights）。
        jev: Option<JevHeads>,
    },
    /// 模板兜底（只提示）。
    Templates { anchors: Vec<[f32; EMBED_DIM]> },
    /// 什么都没有 → L2 对该 profile 不产出分数。
    None,
}

/// 头文件解析中间态（未预计算锚点；embedder 惰性加载的分界）。
enum ParsedHead {
    Linear {
        weights: Vec<f32>,
        bias: f32,
        draft_w: Vec<f32>,
        draft_b: f32,
        fast_path: Option<FastPath>,
        /// jev 多问题头（优先级高于 legacy weights）。
        jev: Option<JevHeads>,
    },
    /// 需模板兜底（load 时才加载 embedder 预计算锚点）。
    Templates(Vec<String>),
    None,
}

/// formal/casual 双头集合（锚点在 load 时预计算，运行期零推理）。
pub struct Heads {
    formal: Head,
    casual: Head,
}

impl Heads {
    /// 无 embedder 时的空集（L1-only 路径）。
    pub fn fallback_only() -> Self {
        Self {
            formal: Head::None,
            casual: Head::None,
        }
    }

    /// 从 models/bge/ 加载两个头。**embedder 惰性创建**（内存优化）：
    /// 只有当某个头需要模板兜底（缺 weights 或维度不符）时才加载 Embedder 预计算锚点；
    /// 若两个头都是训练好的线性头（本项目的默认情形），根本不加载 embedder——
    /// 避免曾经的「Heads::load 白白多加载一次 50MB 会话」的浪费。
    pub fn load(model_dir: &std::path::Path) -> Self {
        // 先只读文件、解析权重，不碰 embedder。
        let formal_parsed = Self::parse_head(&model_dir.join("head-formal.json"));
        let casual_parsed = Self::parse_head(&model_dir.join("head-casual.json"));

        // 惰性 embedder：仅当有头落入模板兜底分支时才创建（Arc 便于两头共享）。
        let mut embedder: Option<std::sync::Arc<super::embedder::Embedder>> = None;
        let mut make = |parsed: ParsedHead| -> Head {
            match parsed {
                ParsedHead::Linear { weights, bias, draft_w, draft_b, fast_path, jev } =>
                    Head::Linear { weights, bias, draft_w, draft_b, fast_path, jev },
                ParsedHead::None => Head::None,
                ParsedHead::Templates(templates) => {
                    if embedder.is_none() {
                        embedder = match super::embedder::Embedder::load(model_dir) {
                            Ok(e) => Some(std::sync::Arc::new(e)),
                            Err(e) => {
                                tracing::warn!(error = %e, "头锚点预计算失败，模板兜底不可用");
                                None
                            }
                        };
                    }
                    match &embedder {
                        Some(emb) => Self::anchors_from(templates, emb),
                        None => Head::None,
                    }
                }
            }
        };
        let formal = make(formal_parsed);
        let casual = make(casual_parsed);
        Self { formal, casual }
    }

    /// 读文件 + 解析权重维度，**不加载 embedder**（判定是否需要模板兜底延后到 load）。
    fn parse_head(path: &std::path::Path) -> ParsedHead {
        let Ok(text) = std::fs::read_to_string(path) else {
            return ParsedHead::None;
        };
        let parsed: HeadFile = match serde_json::from_str(&text) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "头文件非法，按无头处理");
                return ParsedHead::None;
            }
        };
        // 允许三种特征维度：
        // - EMBED_DIM（512）仅草稿塔（引入对象塔之前的旧头）；
        // - 2×EMBED_DIM（1024）草稿塔 ⊕ 对象塔（**可分离**，只能给每对象一个常数偏移）；
        // - 4×EMBED_DIM（2048）再叠加 逐元素积 ⊕ 差（**含交互项**，才能表达
        //   「同一句话 × 不同对象 → 相反判定」）。
        // 其他维度一律降级模板兜底（只提示不拦截），绝不按错误维度打分。
        let dim_ok = [EMBED_DIM, 2 * EMBED_DIM, 4 * EMBED_DIM].contains(&parsed.dim);
        match parsed.weights {
            Some(w) if w.len() == parsed.dim && dim_ok => {
                // fast-path：从 draft-only 权重提取前 EMBED_DIM 维。
                // jev.severity 优先（用于 fast-path draft-only 评分），其次 legacy weights。
                let draft_w = if let Some(ref jev) = parsed.jev {
                    if let Some(ref sev) = jev.severity {
                        if sev.weights.len() >= EMBED_DIM {
                            sev.weights[..EMBED_DIM].to_vec()
                        } else {
                            sev.weights.clone()
                        }
                    } else {
                        w[..EMBED_DIM].to_vec()
                    }
                } else {
                    w[..EMBED_DIM].to_vec()
                };
                let draft_b = if let Some(ref jev) = parsed.jev {
                    if let Some(ref sev) = jev.severity {
                        sev.bias
                    } else {
                        parsed.bias
                    }
                } else {
                    parsed.bias
                };
                ParsedHead::Linear {
                    weights: w,
                    bias: parsed.bias,
                    fast_path: parsed.fast_path,
                    draft_w,
                    draft_b,
                    jev: parsed.jev,
                }
            },
            Some(_) => {
                tracing::warn!(
                    path = %path.display(),
                    dim = parsed.dim,
                    "头维度不符（应为 {} / {} / {}），降级模板兜底",
                    EMBED_DIM,
                    2 * EMBED_DIM,
                    4 * EMBED_DIM
                );
                ParsedHead::Templates(parsed.templates)
            }
            None => ParsedHead::Templates(parsed.templates),
        }
    }

    fn anchors_from(
        templates: Vec<String>,
        embedder: &std::sync::Arc<super::embedder::Embedder>,
    ) -> Head {
        let anchors = templates
            .iter()
            .filter_map(|t| embedder.embed(t).ok())
            .collect::<Vec<_>>();
        if anchors.is_empty() {
            Head::None
        } else {
            Head::Templates { anchors }
        }
    }

    /// 是否有可拦截的训练头（用于 reasons 文案与级别限制）。
    pub fn has_weights(&self, profile: Profile) -> bool {
        matches!(self.head_of(profile), Head::Linear { .. })
    }

    fn head_of(&self, profile: Profile) -> &Head {
        match profile {
            Profile::Formal => &self.formal,
            Profile::Casual => &self.casual,
        }
    }

    /// 该基线所需特征维度（由 head 权重长度决定）。
    /// 无线性头时返回 [`EMBED_DIM`]（仅草稿塔）——调用方据此决定是否拼接对象塔。
    pub fn feature_dim(&self, profile: Profile) -> usize {
        match self.head_of(profile) {
            Head::Linear { weights, jev, .. } => {
                // jev.danger 优先（训练脚本以 jev.danger 权重维度为准）
                if let Some(ref jev_heads) = jev {
                    if let Some(ref danger) = jev_heads.danger {
                        if !danger.weights.is_empty() {
                            return danger.weights.len();
                        }
                    }
                }
                weights.len()
            }
            _ => EMBED_DIM,
        }
    }

    /// 打分：sigmoid(w·x+b)（线性头）或模板最大余弦的补数（兜底）。
    /// 无头/维度不符 → None（调用方 fail-open）。
    ///
    /// ⚠️ `features.len()` 必须与 [`Self::feature_dim`] 一致，否则返回 None：
    /// 若直接 `zip`，1024 维权重配 512 维向量会被静默算成「部分点积」，
    /// 得到看似合理却错误的分数。
    pub fn score(&self, features: &[f32], profile: Profile) -> Option<f32> {
        match self.head_of(profile) {
            Head::Linear { weights, bias, jev, .. } => {
                // jev.danger 优先：替代 legacy weights 打分
                if let Some(ref jev_heads) = jev {
                    if let Some(ref danger) = jev_heads.danger {
                        if danger.weights.len() != features.len() {
                            tracing::warn!(
                                expect = danger.weights.len(),
                                got = features.len(),
                                "jev.danger 维度与特征不符，跳过打分（fail-open）"
                            );
                            return None;
                        }
                        let z: f32 = danger.weights
                            .iter()
                            .zip(features.iter())
                            .map(|(w, x)| w * x)
                            .sum::<f32>()
                            + danger.bias;
                        return Some(1.0 / (1.0 + (-z).exp()));
                    }
                }
                // legacy weights 兜底
                if weights.len() != features.len() {
                    tracing::warn!(
                        expect = weights.len(),
                        got = features.len(),
                        "特征维度与头不符，跳过打分（fail-open）"
                    );
                    return None;
                }
                let z: f32 = weights
                    .iter()
                    .zip(features.iter())
                    .map(|(w, x)| w * x)
                    .sum::<f32>()
                    + bias;
                Some(1.0 / (1.0 + (-z).exp()))
            }
            Head::Templates { anchors } => {
                // 模板兜底只用草稿塔（锚点维度固定 EMBED_DIM）
                if features.len() != EMBED_DIM {
                    return None;
                }
                // 与模板的余弦（embed 与锚点均已 L2 归一化，点积即余弦）
                let best = anchors
                    .iter()
                    .map(|a| {
                        a.iter()
                            .zip(features.iter())
                            .map(|(u, v)| u * v)
                            .sum::<f32>()
                    })
                    .fold(f32::MIN, f32::max);
                if !best.is_finite() {
                    return None;
                }
                Some((1.0 - best).clamp(0.0, 1.0))
            }
            Head::None => None,
        }
    }

    /// draft-only 快速评分：使用前 EMBED_DIM 维权重，跳过对象塔 + 交互项。
    /// 返回 draft-only sigmoid 得分，不涉及对象信息。
    pub fn draft_score(&self, draft: &[f32], profile: Profile) -> Option<f32> {
        match self.head_of(profile) {
            Head::Linear { draft_w, draft_b, .. } => {
                if draft_w.len() != draft.len() {
                    return None;
                }
                let z: f32 = draft_w
                    .iter()
                    .zip(draft.iter())
                    .map(|(w, x)| w * x)
                    .sum::<f32>()
                    + draft_b;
                Some(1.0 / (1.0 + (-z).exp()))
            }
            _ => None,
        }
    }

    /// draft-only 评分 + fast-path 判定。
    /// - 若 head 无 fast_path 配置 → 返回 FullScore，要求完整双塔。
    /// - 若 draft_score 落在 safe_band → ShortCircuitSafe。
    /// - 若 draft_score 落在 block_band → ShortCircuitBlock。
    /// - 否则 → FullScore，调用方需做完整双塔评分。
    pub fn judge_fast(&self, draft: &[f32], profile: Profile) -> ScoreResult {
        let fp = match self.head_of(profile) {
            Head::Linear { fast_path, .. } => fast_path,
            _ => return ScoreResult::FullScore { draft_score: 0.0 },
        };
        let Some(fp) = fp else {
            return ScoreResult::FullScore { draft_score: 0.0 };
        };
        let Some(score) = self.draft_score(draft, profile) else {
            return ScoreResult::FullScore { draft_score: 0.0 };
        };
        if score >= fp.safe_band[0] && score <= fp.safe_band[1] {
            ScoreResult::ShortCircuitSafe
        } else if score >= fp.block_band[0] && score <= fp.block_band[1] {
            ScoreResult::ShortCircuitBlock
        } else {
            ScoreResult::FullScore { draft_score: score }
        }
    }

    /// 状态描述（日志用）。
    pub fn describe(&self) -> &'static str {
        match (&self.formal, &self.casual) {
            (Head::Linear { jev: Some(_), .. }, Head::Linear { jev: Some(_), .. }) => "formal+casual jev 线性头+fast-path",
            (Head::Linear { jev: Some(_), .. }, _) | (_, Head::Linear { jev: Some(_), .. }) => "部分 jev 线性头+其余兜底",
            (Head::Linear { .. }, Head::Linear { .. }) => "formal+casual 线性头+fast-path",
            (Head::Linear { .. }, _) | (_, Head::Linear { .. }) => "部分线性头+其余兜底",
            (Head::Templates { .. }, _) | (_, Head::Templates { .. }) => "模板兜底（只提示不拦截）",
            (Head::None, Head::None) => "无头（仅 L1）",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_head(w: Vec<f32>, b: f32) -> HeadFile {
        HeadFile {
            dim: EMBED_DIM,
            weights: Some(w),
            bias: b,
            templates: vec![],
            fast_path: None,
            jev: None,
        }
    }

    /// UT-SEM-04 伴随：线性头 sigmoid 打分的方向性。
    #[test]
    fn linear_head_scores() {
        let mut w = vec![0.0; EMBED_DIM];
        w[0] = 10.0; // 第一维强正向 → 危险方向
        let hf = linear_head(w.clone(), 0.0);

        let mut x = [0f32; EMBED_DIM];
        x[0] = 1.0;
        // 手工构造 Head::Linear 走公共逻辑
        let head = Head::Linear {
            weights: hf.weights.unwrap(),
            bias: 0.0,
            draft_w: w,
            draft_b: 0.0,
            fast_path: None,
            jev: None,
        };
        let heads = Heads {
            formal: head,
            casual: Head::None,
        };
        let s_pos = heads.score(&x, Profile::Formal).unwrap();
        assert!(s_pos > 0.99, "正方向应接近 1：{s_pos}");

        x[0] = -1.0;
        let s_neg = heads.score(&x, Profile::Formal).unwrap();
        assert!(s_neg < 0.01, "负方向应接近 0：{s_neg}");

        // casual 无头 → None
        assert!(heads.score(&x, Profile::Casual).is_none());
        assert!(!heads.has_weights(Profile::Casual));
    }

    /// 双塔头：1024 维权重正常打分；维度不符 → None（不做静默的部分点积）。
    #[test]
    fn two_tower_head_dim_guard() {
        let dim = 2 * EMBED_DIM;
        let mut w = vec![0.0f32; dim];
        w[EMBED_DIM] = 10.0; // 只让对象塔那一半权重生效
        let heads = Heads {
            formal: Head::Linear {
                weights: w,
                bias: 0.0,
                draft_w: vec![],
                draft_b: 0.0,
                fast_path: None,
                jev: None,
            },
            casual: Head::None,
        };

        // feature_dim 反映权重长度（调用方据此决定是否拼对象塔）
        assert_eq!(heads.feature_dim(Profile::Formal), dim);
        assert_eq!(heads.feature_dim(Profile::Casual), EMBED_DIM);

        // 1024 维特征：对象塔命中 → 接近 1
        let mut f = vec![0.0f32; dim];
        f[EMBED_DIM] = 1.0;
        let s = heads.score(&f, Profile::Formal).unwrap();
        assert!(s > 0.99, "对象塔正方向应接近 1：{s}");

        // 512 维特征（调用方漏拼对象塔）→ None，不得按部分点积出分
        let short = vec![0.0f32; EMBED_DIM];
        assert!(
            heads.score(&short, Profile::Formal).is_none(),
            "维度不符必须返回 None"
        );
    }

    /// 头文件 JSON 往返：weights 有/无两种形态。
    #[test]
    fn head_file_json_roundtrip() {
        let with_w = linear_head(vec![0.1; EMBED_DIM], -0.2);
        let text = serde_json::to_string(&with_w).unwrap();
        let back: HeadFile = serde_json::from_str(&text).unwrap();
        assert!(back.weights.is_some());
        assert_eq!(back.dim, EMBED_DIM);

        let template_only = HeadFile {
            dim: EMBED_DIM,
            weights: None,
            bias: 0.0,
            templates: vec!["正式沟通".into()],
            fast_path: None,
            jev: None,
        };
        let text = serde_json::to_string(&template_only).unwrap();
        let back: HeadFile = serde_json::from_str(&text).unwrap();
        assert!(back.weights.is_none());
        assert_eq!(back.templates.len(), 1);
    }

    /// 兜底模式：与锚点同向 → 低危险分；正交 → 高危险分。
    #[test]
    fn template_fallback_scoring() {
        let mut a = [0f32; EMBED_DIM];
        a[0] = 1.0;
        let heads = Heads {
            formal: Head::Templates { anchors: vec![a] },
            casual: Head::None,
        };
        // 同向（余弦 1）→ 分数 0
        let same = heads.score(&a, Profile::Formal).unwrap();
        assert!(same < 0.01, "{same}");
        // 正交（余弦 0）→ 分数 1
        let mut orth = [0f32; EMBED_DIM];
        orth[1] = 1.0;
        let other = heads.score(&orth, Profile::Formal).unwrap();
        assert!(other > 0.99, "{other}");
        assert!(!heads.has_weights(Profile::Formal), "模板模式不算可拦截头");
    }

    /// jev 多问题头：jev.danger 优先于 legacy weights 打分。
    #[test]
    fn jev_danger_preferred_over_legacy() {
        let dim = 2 * EMBED_DIM;
        // legacy weights: 第一维强正向（w=10）→ 高危险
        let mut legacy_w = vec![0.0f32; dim];
        legacy_w[0] = 10.0;

        // jev.danger weights: 第一维强负向（w=-10）→ 低危险
        let mut danger_w = vec![0.0f32; dim];
        danger_w[0] = -10.0;

        let head = Head::Linear {
            weights: legacy_w,
            bias: 0.0,
            draft_w: vec![],
            draft_b: 0.0,
            fast_path: None,
            jev: Some(JevHeads {
                severity: Some(QuestionHead {
                    weights: vec![0.0; EMBED_DIM],
                    bias: 0.0,
                }),
                danger: Some(QuestionHead {
                    weights: danger_w,
                    bias: 0.0,
                }),
            }),
        };
        let heads = Heads {
            formal: head,
            casual: Head::None,
        };

        let x = vec![1.0f32; dim];
        // jev.danger 权重[0]=-10，输入[0]=1 → z=-10 → score≈0（低危险）
        // 若用 legacy weights（[0]=10），score≈1（高危险）——但 jev.danger 应优先
        let score = heads.score(&x, Profile::Formal).unwrap();
        assert!(
            score < 0.01,
            "jev.danger 应优先打分，预期 score≈0，得到 {score}"
        );
    }
}
