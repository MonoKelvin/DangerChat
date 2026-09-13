use serde::{Deserialize, Serialize};

use crate::import::{collect_images, ImageEntry};
use crate::propagate::{propagate, PropagateRequest, PropagateResult};
use crate::state::{load, save, state_path, AnnoBox, Autosave};
use crate::tags::TagsConfig;

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum UiTagError {
    #[error("{0}")]
    Tags(String),
    #[error("{0}")]
    State(String),
}

impl From<crate::tags::TagsError> for UiTagError {
    fn from(e: crate::tags::TagsError) -> Self {
        UiTagError::Tags(e.to_string())
    }
}

impl From<crate::state::StateError> for UiTagError {
    fn from(e: crate::state::StateError) -> Self {
        UiTagError::State(e.to_string())
    }
}

pub type CmdResult<T> = Result<T, UiTagError>;

#[tauri::command]
pub fn list_images(paths: Vec<String>) -> CmdResult<Vec<ImageEntry>> {
    Ok(collect_images(&paths))
}

#[tauri::command]
pub fn load_tags(path: Option<String>) -> CmdResult<TagsConfig> {
    match path {
        Some(p) => {
            let text = std::fs::read_to_string(&p)
                .map_err(|e| UiTagError::Tags(format!("读取 {p} 失败：{e}")))?;
            Ok(TagsConfig::parse(&text)?)
        }
        None => Ok(TagsConfig::builtin()),
    }
}

#[tauri::command]
pub fn save_state(autosave: Autosave) -> CmdResult<()> {
    save(&autosave, &state_path()).map_err(Into::into)
}

#[tauri::command]
pub fn load_state() -> CmdResult<Option<Autosave>> {
    load(&state_path()).map_err(Into::into)
}

/// 导出请求（P4 实现 zip 写出，DTO 先定下来供前端对齐）。
#[derive(Debug, Clone, Deserialize)]
pub struct ExportImage {
    pub src: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportRequest {
    pub images: Vec<ExportImage>,
    /// 文件名主干（无扩展名）→ 标注框
    pub annos: std::collections::HashMap<String, Vec<AnnoBox>>,
    pub tags: TagsConfig,
    pub dest: String,
}

#[tauri::command]
pub fn export_zip(req: ExportRequest) -> CmdResult<String> {
    let path = crate::export::write_dataset_zip(&req, std::path::Path::new(&req.dest))
        .map_err(UiTagError::State)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn propagate_boxes(req: PropagateRequest) -> CmdResult<Vec<PropagateResult>> {
    propagate(&req).map_err(UiTagError::State)
}
