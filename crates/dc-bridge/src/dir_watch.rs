//! 数据目录监听：models/ 与 datasets/ 轮询扫描，清单变更时推送最新列表给前端。
//!
//! 不用 notify 文件系统钩子：仅两个本地目录、秒级时效即可满足，轮询实现
//! 更短且不引新依赖；变更判定即「清单内容本身」（PartialEq 比较）。

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands::{scan_datasets, scan_models};
use crate::events;
use crate::state::AppState;

pub fn spawn(app: AppHandle) {
    let state: Arc<AppState> = app.state::<Arc<AppState>>().inner().clone();
    std::thread::Builder::new()
        .name("dir-watch".into())
        .spawn(move || {
            let mut last_models: Option<Vec<crate::dto::ModelDto>> = None;
            let mut last_datasets: Option<Vec<crate::dto::DatasetDto>> = None;
            loop {
                if state.is_shutting_down() {
                    return;
                }
                let models = scan_models(&state);
                let datasets = scan_datasets(&state);
                // 训练写 models/ 是「先拷权重后写 model.toml」，半成品目录会被
                // ModelStore::list 跳过，天然只推完整模型
                if last_models.as_ref() != Some(&models) {
                    let _ = app.emit(events::EVENT_MODELS, &models);
                    last_models = Some(models);
                }
                if last_datasets.as_ref() != Some(&datasets) {
                    let _ = app.emit(events::EVENT_DATASETS, &datasets);
                    last_datasets = Some(datasets);
                }
                std::thread::sleep(Duration::from_millis(1500));
            }
        })
        .expect("dir-watch spawn");
}
