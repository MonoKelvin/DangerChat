//! ort 会话管理（§5.5 OnnxSession）。
//!
//! - EP 链 DirectML → CPU（ADR-05：GPU 初始化失败自动回退）；
//! - intra-op 线程 = 2（§2.3 线程纪律）；
//! - 设备解析（ADR-15）：模块默认值 + `device.prefer_gpu=false` 全局强制 CPU。

use ort::ep::{DirectML, ExecutionProvider, CPU};
use ort::session::{Session, SessionOutputs};
use ort::value::Tensor;

use crate::contract::{DevicePref, StageError};

/// layout 模块的默认设备偏好（ADR-15：大模型 GPU 优先）。
pub const LAYOUT_DEFAULT_PREF: DevicePref = DevicePref::Gpu;

/// 解析本轮实际设备：全局强制 CPU > 模块默认。
pub fn resolve_device(pref: DevicePref, prefer_gpu: bool) -> DevicePref {
    if !prefer_gpu {
        DevicePref::Cpu
    } else {
        pref
    }
}

/// 推理会话。`Mutex` 满足 ort `run(&mut)` 签名；锁竞争为零（§2.3 单飞：同一时刻
/// 仅一条流水线在跑），同时显式编码了 DirectML 串行约束。
pub struct InferSession {
    session: std::sync::Mutex<Session>,
    /// 是否请求了 DML EP（算子级回退由 ORT 处理）。
    pub on_gpu: bool,
    pub input_side: u32,
}

impl InferSession {
    /// 从模型文件创建会话。`device` 决定 EP 链；DML 链建立失败自动落到 CPU。
    pub fn load(
        path: &std::path::Path,
        device: DevicePref,
        intra_threads: usize,
    ) -> Result<Self, String> {
        if device != DevicePref::Cpu && DirectML::default().is_available().unwrap_or(false) {
            match Self::build(path, true, intra_threads) {
                Ok(s) => return Ok(s),
                Err(e) => tracing::warn!(error = %e, "DirectML 会话建立失败，回退 CPU"),
            }
        }
        Self::build(path, false, intra_threads)
    }

    fn build(path: &std::path::Path, dml: bool, intra_threads: usize) -> Result<Self, String> {
        let builder = Session::builder().map_err(|e| format!("ort builder 失败：{e}"))?;
        let builder = if dml {
            builder.with_execution_providers([DirectML::default().build(), CPU::default().build()])
        } else {
            builder.with_execution_providers([CPU::default().build()])
        }
        .map_err(|e| format!("EP 配置失败：{e}"))?;
        let session = builder
            .with_intra_threads(intra_threads)
            .map_err(|e| format!("设置 intra 线程失败：{e}"))?
            .commit_from_file(path)
            .map_err(|e| format!("会话建立失败（{}）：{e}", path.display()))?;

        // 从模型签名读输入边长（NCHW 的最后两维应相等且静态；动态则回退 640）
        let input_side = session
            .inputs()
            .first()
            .and_then(|o| {
                if let ort::value::ValueType::Tensor { shape, .. } = o.dtype() {
                    shape.last().copied()
                } else {
                    None
                }
            })
            .and_then(|d| u32::try_from(d).ok())
            .filter(|&d| (16..=4096).contains(&d))
            .unwrap_or(640);

        Ok(Self {
            session: std::sync::Mutex::new(session),
            on_gpu: dml,
            input_side,
        })
    }

    /// 跑一次 YOLO 推理：`1×3×side×side` f32 CHW → 解码数据。
    ///
    /// 返回 `(输出数据, anchors, classes)`，输出统一按 `(4+nc) × anchors` 行主序。
    pub fn run(&self, chw: Vec<f32>, side: u32) -> Result<(Vec<f32>, usize, usize), StageError> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| StageError::Recoverable("layout 会话锁中毒".into()))?;
        let shape = vec![1usize, 3, side as usize, side as usize];
        let input = Tensor::from_array((shape, chw.into_boxed_slice()))
            .map_err(|e| StageError::Recoverable(format!("输入张量构造失败：{e}")))?;
        let outputs: SessionOutputs = session
            .run(ort::inputs!["images" => input])
            .map_err(|e| StageError::Recoverable(format!("layout 推理失败：{e}")))?;

        let (mut flat, mut dim0, mut dim1) = (Vec::new(), 0usize, 0usize);
        for (_, v) in outputs.iter() {
            let Ok((shape, data)) = v.try_extract_tensor::<f32>() else {
                continue;
            };
            if shape.len() == 3 {
                dim0 = shape[1] as usize;
                dim1 = shape[2] as usize;
                flat = data.to_vec();
                break;
            }
        }
        if flat.is_empty() {
            return Err(StageError::Recoverable("layout 输出张量缺失".into()));
        }
        // 形状语义：v8/v11 标准导出是 (4+nc)×anchors（如 8×8400），(4+nc) 恒远小于 anchors；
        // 若 dim0 > dim1 则是 anchors×(4+nc) 的转置导出，转回行主序统一处理。
        if dim0 > dim1 {
            let (rows, cols) = (dim1, dim0);
            let mut t = vec![0f32; flat.len()];
            for r in 0..rows {
                for c in 0..cols {
                    t[c * rows + r] = flat[r * cols + c];
                }
            }
            return Ok((t, rows, cols.saturating_sub(4)));
        }
        Ok((flat, dim1, dim0.saturating_sub(4)))
    }
}
