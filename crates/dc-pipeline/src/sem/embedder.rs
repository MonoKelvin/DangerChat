//! BGE 嵌入（§5.7 Embedder，ADR-14）。
//!
//! `tokenizer.json` 分词 → ONNX int8 量化模型（三输入 i64）→ last_hidden_state
//! → **标记位置读取**（jev 风格）：在 [CLS] 后插入 [unused1]/[unused2] 标记，
//! 读取位置 1 的隐藏状态（L2 归一化）。对比 mean-pooling，标记位置读取能保持
//! 特定语义片段的信息不被稀释（§5.7-jev）。
//! 设备（ADR-15）：sem 默认 **CPU**（短草稿算量小，GPU 派发开销反而更慢）。

use std::path::Path;

use ort::session::{Session, SessionOutputs};
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::contract::StageError;

/// 嵌入维度（bge-small-zh：512）。编码器换档后由此常量统一传播到
/// head 权重维度校验与双塔特征装配（sem/head.rs、sem/mod.rs 均引用本常量）。
pub const EMBED_DIM: usize = 512;
/// token 上限（草稿文本按 128 截断）。
const MAX_SEQ: usize = 128;
/// [unused1] 标记 ID：draft 塔嵌入用。
const MARKER_DRAFT: i64 = 1;
/// [unused2] 标记 ID：object 塔嵌入用。
const MARKER_OBJECT: i64 = 2;

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
        // 低内存配置（bge-small int8 权重 24MB，目标：运行时不常驻大块 RSS）：
        // 1) device_allocated_initializers：初始化权重不被 arena 额外复制一份，单副本驻留；
        // 2) 关闭 mem_pattern：动态序列长度下预规划分配无收益，反而预留冗余内存；
        // 3) 关闭 intra_op 线程 busy-spin：空闲不空转，贴合「无感知」CPU 目标。
        let builder = builder
            .with_device_allocated_initializers()
            .and_then(|b| b.with_memory_pattern(false))
            .and_then(|b| b.with_intra_op_spinning(false))
            .map_err(|e| format!("内存配置失败：{e}"))?;
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

    /// 标记位置读取嵌入：[CLS] marker text... → 位置 1 的隐藏状态（L2 归一化）。
    /// marker_id = MARKER_DRAFT（1）用于草稿塔，MARKER_OBJECT（2）用于对象塔。
    fn embed_at_marker(&self, text: &str, marker_id: i64) -> Result<[f32; EMBED_DIM], StageError> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| StageError::Recoverable(format!("分词失败：{e}")))?;
        // [CLS] + marker + 其余内容（跳过 enc 的 [CLS]，取 [1:127]）
        let mut ids: Vec<i64> = vec![101, marker_id];
        ids.extend(
            enc.get_ids()
                .iter()
                .skip(1)
                .take(MAX_SEQ - 2)
                .map(|&v| v as i64),
        );
        if ids.is_empty() {
            return Err(StageError::Recoverable("分词结果为空".into()));
        }
        let seq = ids.len();
        let mask = vec![1i64; seq];
        let types = vec![0i64; seq];

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

        let first = outputs
            .iter()
            .next()
            .ok_or_else(|| StageError::Recoverable("语义输出缺失".into()))?;
        let (_, hidden) = first
            .1
            .try_extract_tensor::<f32>()
            .map_err(|e| StageError::Recoverable(format!("语义输出解包失败：{e}")))?;

        // 位置 1（marker 位置）的隐藏状态
        let mut vec = [0f32; EMBED_DIM];
        for d in 0..EMBED_DIM {
            vec[d] = hidden[1 * EMBED_DIM + d];
        }
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm < 1e-9 {
            return Err(StageError::Recoverable("嵌入退化（零向量）".into()));
        }
        for v in vec.iter_mut() {
            *v /= norm;
        }
        Ok(vec)
    }

    /// 草稿塔嵌入：[CLS] [unused1] {text}。
    pub fn embed_draft(&self, text: &str) -> Result<[f32; EMBED_DIM], StageError> {
        self.embed_at_marker(text, MARKER_DRAFT)
    }

    /// 对象塔嵌入：[CLS] [unused2] {text}。
    pub fn embed_object(&self, text: &str) -> Result<[f32; EMBED_DIM], StageError> {
        self.embed_at_marker(text, MARKER_OBJECT)
    }

    /// 文本 → L2 归一化句向量（`[EMBED_DIM]`）。
    /// 兼容旧调用路径（模板兜底）。
    pub fn embed(&self, text: &str) -> Result<[f32; EMBED_DIM], StageError> {
        self.embed_at_marker(text, MARKER_DRAFT)
    }
}

fn tensor_err(e: ort::Error) -> StageError {
    StageError::Recoverable(format!("语义输入张量构造失败：{e}"))
}
