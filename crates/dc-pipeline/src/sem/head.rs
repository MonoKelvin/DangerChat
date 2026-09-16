//! 场景头（§5.7 ScenarioStore，ADR-14）：预训练线性头 + 模板相似度兜底。
//!
//! 头文件 `models/bge/head-{formal,casual}.json`：
//! ```json
//! {
//!   "dim": 512,
//!   "weights": [512 个 f32],   // 可省略 → 纯模板兜底（只提示不拦截）
//!   "bias": 0.0,
//!   "templates": ["正式场景描述…", "..."]   // 兜底模式的相似度锚点
//! }
//! ```
//! - 有 weights：`score = sigmoid(w·embed + b)`（可拦截）；
//! - 无 weights：与模板最大余弦的补数当分数，**输出只用于提示**（由调用方
//!   `has_weights` 判定后限制级别，不 Block——ADR-14 的降级语义）。

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
}

#[derive(Debug, Clone)]
enum Head {
    /// 训练过的线性头（可拦截）。
    Linear { weights: Vec<f32>, bias: f32 },
    /// 模板兜底（只提示）。
    Templates { anchors: Vec<[f32; EMBED_DIM]> },
    /// 什么都没有 → L2 对该 profile 不产出分数。
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

    /// 从 models/bge/ 加载两个头 + 模板锚点（锚点预计算，运行期零推理）。
    pub fn load(model_dir: &std::path::Path) -> Self {
        let embedder = std::sync::Arc::new(match super::embedder::Embedder::load(model_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "头锚点预计算失败，模板兜底不可用");
                return Self::fallback_only();
            }
        });

        let formal = Self::load_head(&model_dir.join("head-formal.json"), &embedder);
        let casual = Self::load_head(&model_dir.join("head-casual.json"), &embedder);
        Self { formal, casual }
    }

    fn load_head(
        path: &std::path::Path,
        embedder: &std::sync::Arc<super::embedder::Embedder>,
    ) -> Head {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Head::None;
        };
        let parsed: HeadFile = match serde_json::from_str(&text) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "头文件非法，按无头处理");
                return Head::None;
            }
        };
        match parsed.weights {
            Some(w) if w.len() == parsed.dim && parsed.dim == EMBED_DIM => Head::Linear {
                weights: w,
                bias: parsed.bias,
            },
            Some(_) => {
                tracing::warn!(path = %path.display(), "头维度不符（dim={}），降级模板兜底", parsed.dim);
                Self::anchors_from(parsed.templates, embedder)
            }
            None => Self::anchors_from(parsed.templates, embedder),
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

    /// 打分：sigmoid(w·x+b)（线性头）或模板最大余弦的补数（兜底）。
    /// 无头/维度不符 → None（调用方 fail-open）。
    pub fn score(&self, embed: &[f32; EMBED_DIM], profile: Profile) -> Option<f32> {
        match self.head_of(profile) {
            Head::Linear { weights, bias } => {
                let z: f32 = weights
                    .iter()
                    .zip(embed.iter())
                    .map(|(w, x)| w * x)
                    .sum::<f32>()
                    + bias;
                Some(1.0 / (1.0 + (-z).exp()))
            }
            Head::Templates { anchors } => {
                // 与模板的余弦（embed 与锚点均已 L2 归一化，点积即余弦）
                let best = anchors
                    .iter()
                    .map(|a| a.iter().zip(embed.iter()).map(|(u, v)| u * v).sum::<f32>())
                    .fold(f32::MIN, f32::max);
                if !best.is_finite() {
                    return None;
                }
                Some((1.0 - best).clamp(0.0, 1.0))
            }
            Head::None => None,
        }
    }

    /// 状态描述（日志用）。
    pub fn describe(&self) -> &'static str {
        match (&self.formal, &self.casual) {
            (Head::Linear { .. }, Head::Linear { .. }) => "formal+casual 线性头",
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
        }
    }

    /// UT-SEM-04 伴随：线性头 sigmoid 打分的方向性。
    #[test]
    fn linear_head_scores() {
        let mut w = vec![0.0; EMBED_DIM];
        w[0] = 10.0; // 第一维强正向 → 危险方向
        let hf = linear_head(w, 0.0);

        let mut x = [0f32; EMBED_DIM];
        x[0] = 1.0;
        // 手工构造 Head::Linear 走公共逻辑
        let head = Head::Linear {
            weights: hf.weights.unwrap(),
            bias: 0.0,
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
}
