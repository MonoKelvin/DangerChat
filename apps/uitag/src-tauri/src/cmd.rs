use serde::{Deserialize, Serialize};

use crate::import::ImageEntry;
use crate::propagate::{propagate, PropagateRequest, PropagateResult};
use crate::state::{self, AnnoBox, Autosave};
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

/// 打开工作目录：图片清单 + 目录下 annotations.json 的标注（无则创建空文件）。
#[tauri::command]
pub fn open_workspace(dir: String) -> CmdResult<Workspace> {
    let (images, autosave) =
        state::open_workspace(std::path::Path::new(&dir)).map_err(UiTagError::from)?;
    Ok(Workspace { images, autosave })
}

#[derive(Debug, Clone, Serialize)]
pub struct Workspace {
    pub images: Vec<ImageEntry>,
    pub autosave: Autosave,
}

#[tauri::command]
pub fn save_state(dir: String, autosave: Autosave) -> CmdResult<()> {
    state::save_workspace(&autosave, std::path::Path::new(&dir)).map_err(Into::into)
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

/// 在系统文件管理器中定位该文件（Windows 资源管理器选中态）。
#[tauri::command]
pub fn reveal_path(path: String) -> CmdResult<()> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map(|_| ());
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .args(["-R", &path])
        .spawn()
        .map(|_| ());
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(
            std::path::Path::new(&path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        )
        .spawn()
        .map(|_| ());
    result.map_err(|e| UiTagError::State(format!("打开文件管理器失败：{e}")))
}

/// 识别当前图片的分割线（后台线程，结果缓存）。
#[tauri::command]
pub async fn detect_lines(path: String, min_run: u32) -> CmdResult<Vec<crate::detect::SnapLine>> {
    tauri::async_runtime::spawn_blocking(move || crate::detect::detect(&path, min_run))
        .await
        .map_err(|e| UiTagError::State(format!("识别任务失败：{e}")))?
        .map_err(UiTagError::State)
}
