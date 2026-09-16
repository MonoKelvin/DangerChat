//! bootstrap.json 管理（数据目录指针 + 启动时配置）。
//!
//! 文件位置：**固定在 Roaming 目录**（`%APPDATA%\com.monostudio.dangerchat\bootstrap.json`），
//! 不跟随数据目录迁移——这样用户换目录后重启即生效，无需先找到旧目录删指针。
//!
//! 迁移策略：
//! - 旧指针文件 `data_dir.txt` 被忽略且删除（不做导入，接受数据丢失）。
//! - 首次启动时 `bootstrap.json` 不存在 → 创建默认配置，数据目录 = Roaming。

use std::path::{Path, PathBuf};

use dc_core::{config_doc::JsonStore, ConfigError};
use serde::{Deserialize, Serialize};

/// bootstrap.json 顶层结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    /// 数据目录路径（config.json / rules.json / models/ / logs/ 等的父目录）
    pub data_dir: PathBuf,
    /// 循环图片目录游标（1-based，下次写入哪个目录）
    #[serde(default = "default_images_cursor")]
    pub images_cursor: u64,
}

fn default_images_cursor() -> u64 {
    1
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::new(), // 会被 bootstrap 填充为 Roaming 目录
            images_cursor: 1,
        }
    }
}

/// 加载或创建 bootstrap.json（固定位置：Roaming 目录）。
///
/// 返回：(bootstrap store, 实际数据目录路径)
pub fn load_or_create(
    roaming_dir: &Path,
) -> Result<(JsonStore<BootstrapConfig>, PathBuf), ConfigError> {
    let bootstrap_path = roaming_dir.join("bootstrap.json");

    let store = JsonStore::open("bootstrap", &bootstrap_path)?;

    // 如果是首次创建，用 Roaming 目录初始化
    if store.is_fresh() {
        store.write(|cfg: &mut BootstrapConfig| {
            cfg.data_dir = roaming_dir.to_path_buf();
        })?;
    }

    let data_dir = store.with(|cfg| cfg.data_dir.clone());

    // 数据目录为空 → 回落 Roaming（兜底保护）
    let data_dir = if data_dir.as_os_str().is_empty() {
        roaming_dir.to_path_buf()
    } else {
        data_dir
    };

    Ok((store, data_dir))
}

/// 删除旧指针文件 data_dir.txt（迁移清理，忽略错误）。
pub fn cleanup_legacy_pointer(roaming_dir: &Path) {
    let legacy = roaming_dir.join("data_dir.txt");
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
        tracing::info!(path = %legacy.display(), "已删除旧指针文件 data_dir.txt");
    }
}
