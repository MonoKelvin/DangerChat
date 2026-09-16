//! 模型管理（设计文档 §5.1 `ModelStore`、§5.5 模型导入校验）。
//!
//! 目录约定（§7 `models/`）：
//!
//! ```text
//! models/
//!   dc-layout-wechat/        # 一个模型一个目录
//!     model.toml              # 元信息：kind / version / file / input_size / classes
//!     yolo11n-dc-layout-wechat-v1.onnx
//! ```
//!
//! 内置模型随发布物携带；用户自训练模型经 `import_model` 导入（M6），导入时校验
//! 类别集合必须覆盖当前启用的标签（§5.5 UT-LAY-05），不满足直接拒绝。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// 区域划分 YOLO
    Layout,
    /// 语义句向量
    Sem,
    /// OCR（det/cls/rec 三件套中的一个）
    Ocr,
}

impl ModelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelKind::Layout => "layout",
            ModelKind::Sem => "sem",
            ModelKind::Ocr => "ocr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "layout" => Some(ModelKind::Layout),
            "sem" => Some(ModelKind::Sem),
            "ocr" => Some(ModelKind::Ocr),
            _ => None,
        }
    }
}

/// `model.toml` 内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelMeta {
    pub kind: String,
    /// 模型文件相对路径（同目录下的文件名）。
    pub file: String,
    #[serde(default)]
    pub version: String,
    /// 输入边长（YOLO 网络输入，默认 640）；OCR/语义模型可为空。
    #[serde(default)]
    pub input_size: Option<u32>,
    /// 类别表（layout 必需；顺序即 YOLO 类别 index 顺序）。
    #[serde(default)]
    pub classes: Vec<String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    /// 目录名，即调用方引用的模型名。
    pub name: String,
    pub dir: PathBuf,
    pub meta: ModelMeta,
    /// 权重文件字节数（不含 model.toml）。
    pub size_bytes: u64,
}

impl ModelInfo {
    pub fn kind(&self) -> Option<ModelKind> {
        ModelKind::parse(&self.meta.kind)
    }

    pub fn weight_path(&self) -> PathBuf {
        self.dir.join(&self.meta.file)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("model dir not found: {0}")]
    NotFound(PathBuf),
    #[error("model.toml missing in {0}")]
    MetaMissing(PathBuf),
    #[error("model.toml invalid in {0}: {1}")]
    MetaInvalid(PathBuf, String),
    #[error("model kind mismatch: expected {expected}, got {actual}")]
    KindMismatch { expected: String, actual: String },
    #[error("model weights missing: {0}")]
    WeightMissing(PathBuf),
    #[error("model classes do not cover enabled tags, missing: {missing:?}")]
    ClassesMissing { missing: Vec<String> },
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// `models/` 目录管理器。
#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir_of(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 扫描全部模型目录（`model.toml` 缺失/非法的目录跳过并记日志，不中断扫描）。
    pub fn list(&self) -> Vec<ModelInfo> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            match self.load_dir(&dir) {
                Ok(info) => out.push(info),
                Err(err) => tracing::warn!(dir = %dir.display(), error = %err, "跳过非法模型目录"),
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn get(&self, name: &str) -> Option<ModelInfo> {
        self.list().into_iter().find(|m| m.name == name)
    }

    /// 读取某目录的模型元信息。
    pub fn load_dir(&self, dir: &Path) -> Result<ModelInfo, ModelError> {
        let meta_path = dir.join("model.toml");
        if !meta_path.is_file() {
            return Err(if dir.is_dir() {
                ModelError::MetaMissing(dir.to_path_buf())
            } else {
                ModelError::NotFound(dir.to_path_buf())
            });
        }
        let text = fs::read_to_string(&meta_path).map_err(|source| ModelError::Io {
            path: meta_path.clone(),
            source,
        })?;
        let meta: ModelMeta = toml::from_str(&text)
            .map_err(|e| ModelError::MetaInvalid(dir.to_path_buf(), e.to_string()))?;
        if meta.file.trim().is_empty() {
            return Err(ModelError::MetaInvalid(
                dir.to_path_buf(),
                "`file` 不能为空".to_string(),
            ));
        }
        let weight = dir.join(&meta.file);
        if !weight.is_file() {
            return Err(ModelError::WeightMissing(weight));
        }
        let size_bytes = fs::metadata(&weight).map(|m| m.len()).unwrap_or(0);
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        Ok(ModelInfo {
            name,
            dir: dir.to_path_buf(),
            meta,
            size_bytes,
        })
    }

    /// 校验类别集合是否覆盖启用标签（§5.5：不覆盖则拒绝导入/加载）。
    pub fn validate_classes(meta: &ModelMeta, required: &[String]) -> Result<(), ModelError> {
        let missing: Vec<String> = required
            .iter()
            .filter(|tag| !meta.classes.iter().any(|c| c == *tag))
            .cloned()
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ModelError::ClassesMissing { missing })
        }
    }

    /// 导入模型：`src` 可以是模型目录，也可以是「一个 .onnx + 同级 model.toml」所在目录。
    ///
    /// 校验链：kind 匹配 → `model.toml` 合法 → 权重存在 → （layout）类别覆盖。
    /// 通过后整体复制到 `models/<name>/`。
    pub fn import(
        &self,
        kind: ModelKind,
        src: &Path,
        required_classes: &[String],
    ) -> Result<ModelInfo, ModelError> {
        let src_info = self.load_dir(src)?;
        let actual = src_info.kind().ok_or_else(|| {
            ModelError::MetaInvalid(
                src.to_path_buf(),
                format!("未知 kind `{}`", src_info.meta.kind),
            )
        })?;
        if actual != kind {
            return Err(ModelError::KindMismatch {
                expected: kind.as_str().to_string(),
                actual: actual.as_str().to_string(),
            });
        }
        if kind == ModelKind::Layout {
            if src_info.meta.classes.is_empty() {
                return Err(ModelError::MetaInvalid(
                    src.to_path_buf(),
                    "layout 模型必须声明 classes".to_string(),
                ));
            }
            Self::validate_classes(&src_info.meta, required_classes)?;
        }

        let name = if src_info.name.is_empty() {
            format!("{}-{}", kind.as_str(), src_info.meta.version)
        } else {
            src_info.name.clone()
        };
        let dst = self.dir_of(&name);
        if dst == src {
            return self.load_dir(&dst);
        }
        fs::create_dir_all(&dst).map_err(|source| ModelError::Io {
            path: dst.clone(),
            source,
        })?;
        for entry in fs::read_dir(src)
            .map_err(|source| ModelError::Io {
                path: src.to_path_buf(),
                source,
            })?
            .flatten()
        {
            let from = entry.path();
            if !from.is_file() {
                continue;
            }
            let to = dst.join(entry.file_name());
            fs::copy(&from, &to).map_err(|source| ModelError::Io {
                path: to.clone(),
                source,
            })?;
        }
        self.load_dir(&dst)
    }
}
