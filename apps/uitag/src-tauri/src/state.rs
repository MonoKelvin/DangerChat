use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 自动保存的标注现场：按图片路径索引的标注框集合 + 元信息。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Autosave {
    /// 图片绝对路径 → 标注框
    pub annos: HashMap<String, Vec<AnnoBox>>,
}

/// 标注框（像素坐标，原图像素系——导出时才做归一化）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnnoBox {
    pub tag: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("状态文件读写失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("状态文件解析失败：{0}")]
    Parse(String),
}

/// 自动保存文件路径：固定在可执行文件同级的 `annotations.json`（开发期即 target/debug，
/// 与构建产物共存，不会污染仓库——target/ 在 .gitignore）。
pub fn state_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("annotations.json")
}

pub fn save(autosave: &Autosave, path: &PathBuf) -> Result<(), StateError> {
    let text = serde_json::to_string_pretty(autosave)
        .map_err(|e| StateError::Parse(e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

pub fn load(path: &PathBuf) -> Result<Option<Autosave>, StateError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let a: Autosave = serde_json::from_str(&text)
                .map_err(|e| StateError::Parse(e.to_string()))?;
            Ok(Some(a))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("annotations.json");
        let mut a = Autosave::default();
        a.annos.insert(
            "C:\\img\\a.png".into(),
            vec![AnnoBox { tag: "msg_input".into(), x: 10.0, y: 20.0, w: 100.0, h: 50.0 }],
        );
        save(&a, &path).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded, a);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(&tmp.path().join("nope.json")).unwrap().is_none());
    }
}
