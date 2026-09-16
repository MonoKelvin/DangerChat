//! BGE 嵌入（§5.7 Embedder，ADR-14）。
//!
//! `tokenizer.json` 分词 → ONNX int8 量化模型（三输入 i64）→ last_hidden_state
//! → **mask 加权 mean-pooling + L2 归一化**（BGE 官方做法）。
//! 设备（ADR-15）：sem 默认 **CPU**（短草稿算量小，GPU 派发开销反而更慢）。

use std::path::Path;

use ort::session::{Session, SessionOutputs};
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::contract::StageError;

/// 嵌入维度（bge-small-zh）。
pub const EMBED_DIM: usize = 512;
/// token 上限（草稿文本按 128 截断，M3 实测该长度 CPU 9.1ms）。
const MAX_SEQ: usize = 128;

pub struct Embedder {
    session: std::sync::Mutex<Session>,
    tokenizer: Tokenizer,
}

impl Embedder {
    /// 从模型目录（`models/bge/`）加载：model.toml 指定的 onnx + tokenizer.json。
    pub fn load(model_dir: &Path) -> Result<Self, String> {
        let tokenizer_path = model_dir.join("tokenizer.json");
        if !tokenizer_path.is_file() {
            return Err(format!("tokenizer 缺失：{}", tokenizer_path.display()));
        }
        // 权重：model.toml 优先；回退 model_quantized.onnx / model.onnx
        let weight = ["model_quantized.onnx", "model.onnx"]
            .iter()
            .map(|f| model_dir.join(f))
            .find(|p| p.is_file())
            .ok_or_else(|| format!("语义模型权重缺失：{}", model_dir.display()))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("tokenizer 加载失败：{e}"))?;
        // CPU（ADR-15：sem 默认 CPU；短文本 GPU 派发开销占主导）
        let builder =
            ort::session::Session::builder().map_err(|e| format!("ort builder 失败：{e}"))?;
        let builder = builder
            .with_execution_providers([ort::ep::CPU::default().build()])
            .map_err(|e| format!("EP 配置失败：{e}"))?;
        let session = builder
            .with_intra_threads(2)
            .map_err(|e| format!("设置 intra 线程失败：{e}"))?
            .commit_from_file(&weight)
            .map_err(|e| format!("语义模型加载失败（{}）：{e}", weight.display()))?;
        tracing::info!(model = %weight.display(), device = "cpu", "语义模型已加载");
        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokenizer,
        })
    }

    /// 文本 → L2 归一化句向量（`[EMBED_DIM]`）。
    pub fn embed(&self, text: &str) -> Result<[f32; EMBED_DIM], StageError> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| StageError::Recoverable(format!("分词失败：{e}")))?;
        let ids: Vec<i64> = enc
            .get_ids()
            .iter()
            .map(|&v| v as i64)
            .take(MAX_SEQ)
            .collect();
        let mask: Vec<i64> = enc
            .get_attention_mask()
            .iter()
            .map(|&v| v as i64)
            .take(MAX_SEQ)
            .collect();
        if ids.is_empty() {
            return Err(StageError::Recoverable("分词结果为空".into()));
        }
        let types = vec![0i64; ids.len()];
        let seq = ids.len();

        let mut session = self
            .session
            .lock()
            .map_err(|_| StageError::Recoverable("语义会话锁中毒".into()))?;
        let outputs: SessionOutputs = session
            .run(ort::inputs! {
                "input_ids" => Tensor::from_array((vec![1, seq], ids.into_boxed_slice())).map_err(tensor_err)?,
                "attention_mask" => Tensor::from_array((vec![1, seq], mask.clone().into_boxed_slice())).map_err(tensor_err)?,
                "token_type_ids" => Tensor::from_array((vec![1, seq], types.into_boxed_slice())).map_err(tensor_err)?,
            })
            .map_err(|e| StageError::Recoverable(format!("语义推理失败：{e}")))?;

        // last_hidden_state: [1, seq, 512]
        let first = outputs
            .iter()
            .next()
            .ok_or_else(|| StageError::Recoverable("语义输出缺失".into()))?;
        let (_, hidden) = first
            .1
            .try_extract_tensor::<f32>()
            .map_err(|e| StageError::Recoverable(format!("语义输出解包失败：{e}")))?;

        // mask 加权 mean-pooling + L2 归一化（BGE 官方做法；与 calibrate_sem.py 对齐）
        let mut vec = [0f32; EMBED_DIM];
        let mut total = 0f32;
        for t in 0..seq {
            let m = mask[t] as f32;
            total += m;
            for d in 0..EMBED_DIM {
                vec[d] += hidden[t * EMBED_DIM + d] * m;
            }
        }
        let denom = total.max(1e-9);
        for v in vec.iter_mut() {
            *v /= denom; // mean
        }
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm < 1e-9 {
            return Err(StageError::Recoverable("嵌入退化（零向量）".into()));
        }
        for v in vec.iter_mut() {
            *v /= norm; // L2
        }
        Ok(vec)
    }
}

fn tensor_err(e: ort::Error) -> StageError {
    StageError::Recoverable(format!("语义输入张量构造失败：{e}"))
}
