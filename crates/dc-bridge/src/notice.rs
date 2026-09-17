//! 首启风险告知的同意状态（FR-UI-08 / 合规文档 §5.1，**P0 强制项**）。
//!
//! 状态存 `config.json` 的非 schema 键 `shell.notice_agreed`
//! （[`ConfigCenter::set_unmanaged_batch`]），与 `window.*` 同范式：同一套原子写与
//! 损坏自愈、不进设置页表单、不被 `finalize()` 丢弃。
//!
//! **为什么不用 localStorage**：主窗口是 `visible: false` 的静默启动，后端必须在
//! `show()` **之前**就知道「首启是否需要弹告知页」；而 localStorage 只存在于 WebView，
//! 后端读不到，且其生命周期绑 WebView 数据目录、与用户可迁移的数据目录不同源
//! （清缓存即丢，会出现「已同意又要求同意」）。放 config.json 则与数据目录同源。
//!
//! 读取方向一律保守：键缺失（首次运行）、类型不符、读取异常都视为**未同意**。

use dc_core::{ConfigCenter, ConfigError, ConfigValue};

/// 同意状态键。非 schema：设置页不渲染，也没有模块订阅它。
pub const KEY_NOTICE_AGREED: &str = "shell.notice_agreed";

/// 是否已勾选同意。缺失或类型不符一律 `false`（保守方向）。
pub fn agreed(config: &ConfigCenter) -> bool {
    config
        .snapshot()
        .get(KEY_NOTICE_AGREED)
        .and_then(ConfigValue::as_bool)
        .unwrap_or(false)
}

/// 记录「已同意」并原子落盘。
///
/// 失败必须上抛给前端——静默放过会出现「界面进了设置页、后端仍认为未同意」
/// 的分裂状态（拦截被门控关闭，而用户以为已生效）。
pub fn ack(config: &ConfigCenter) -> Result<(), ConfigError> {
    config.set_unmanaged_batch(vec![(
        KEY_NOTICE_AGREED.to_string(),
        ConfigValue::Bool(true),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_core::ConfigField;

    /// 建一个「已 finalize」的配置中心：磁盘值经 finalize 才会并入快照，
    /// 未 finalize 时快照为空 —— 测试必须走完整生命周期，否则永远读到 false。
    fn center(disk: Option<&str>) -> (tempfile::TempDir, ConfigCenter) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        if let Some(text) = disk {
            std::fs::write(&path, text).unwrap();
        }
        let config = ConfigCenter::open(&path).unwrap();
        // 一个 schema 键，确保 finalize 真的跑过（贴近 bootstrap 的真实调用序）
        config
            .register_module(
                "guard",
                vec![ConfigField::new(
                    "guard.enabled",
                    dc_core::ConfigType::Bool,
                    ConfigValue::Bool(true),
                    "启用守护",
                    "guard",
                    "guard",
                )],
            )
            .unwrap();
        config.finalize().unwrap();
        (dir, config)
    }

    /// 首次运行（无键）→ 未同意。这是「必须弹告知页」的判定输入。
    #[test]
    fn absent_key_means_not_agreed() {
        let (_dir, config) = center(None);
        assert!(!agreed(&config), "首启无键必须视为未同意");
    }

    /// 落盘后重新打开 → 仍为已同意（持久化跨进程有效）。
    #[test]
    fn ack_persists_across_reopen() {
        let (dir, config) = center(None);
        ack(&config).unwrap();
        assert!(agreed(&config));
        // 落盘形态是嵌套对象（点号键经 insert_json_path 展开）
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        assert!(
            text.contains("\"notice_agreed\": true"),
            "应落盘为 shell.notice_agreed 嵌套键，实际：{text}"
        );

        let reopened = ConfigCenter::open(dir.path().join("config.json")).unwrap();
        reopened.finalize().unwrap();
        assert!(agreed(&reopened), "重新打开后同意状态必须保留");
    }

    /// 非 schema 键不得被 finalize 丢弃（否则同意状态每次启动都被重置）。
    #[test]
    fn unmanaged_key_survives_finalize() {
        let (dir, config) = center(Some(r#"{"shell":{"notice_agreed":true}}"#));
        assert!(agreed(&config), "finalize 必须保留磁盘上的非 schema 键");
        // finalize 会规范化落盘，键仍在
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        assert!(text.contains("\"notice_agreed\": true"), "实际：{text}");
    }

    /// 类型不符（被手改成字符串）→ 保守视为未同意，不得 panic。
    #[test]
    fn wrong_type_is_not_agreed() {
        let (_dir, config) = center(Some(r#"{"shell":{"notice_agreed":"yes"}}"#));
        assert!(!agreed(&config), "类型不符必须保守视为未同意");
    }
}
