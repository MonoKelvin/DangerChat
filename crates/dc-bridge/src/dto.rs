//! 跨进程 DTO（§5.9）。**draft_text 只进弹窗事件**（FR-BRG-02 弹窗要展示被拦消息），
//! 任何 stats/status 载荷都只含元数据（FR-BRG-05）。

use serde::{Deserialize, Serialize};

use dc_pipeline::verdict::{Verdict, VerdictLevel};

/// 弹窗载荷（`alert://blocked`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertPayload {
    pub level: String,
    pub score: f32,
    pub reasons: Vec<String>,
    pub chat_target: Option<String>,
    /// 命中场景的展示名（正式/个人/自定义名；弹窗标签用）。
    pub scene_name: Option<String>,
    /// 聊天窗口最近对话摘要（弹窗辅助判断）。
    pub chat_context: Option<String>,
    /// 被拦截的消息文本（仅弹窗内存展示；不落日志）。
    pub draft_text: String,
    /// 指纹（日志关联用）。
    pub draft_fingerprint: u64,
    pub draft_epoch: u64,
    /// 倒计时秒数（默认动作见 config `alert.timeout_action`）。
    pub countdown_secs: u64,
}

impl AlertPayload {
    pub fn from_verdict(v: &Verdict, countdown_secs: u64) -> Self {
        Self {
            level: level_str(v.level).to_string(),
            score: v.score,
            reasons: v.reasons.clone(),
            chat_target: v.chat_target.clone(),
            scene_name: v.scene_name.clone(),
            chat_context: v.chat_context.clone(),
            draft_text: v.draft_text.clone().unwrap_or_default(),
            draft_fingerprint: v.draft_fingerprint,
            draft_epoch: v.draft_epoch,
            countdown_secs,
        }
    }
}

/// 状态载荷（`guard://status`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPayload {
    pub state: String,
    pub target_process: String,
    /// 目标窗口是否已发现（Guard 是否在跑）。
    pub target_found: bool,
}

/// 统计载荷（`guard://stats`，无消息原文）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatsPayload {
    /// 今日拦截次数（按日累加，跨日清零）。
    pub today_blocked: u64,
    /// 弹窗显示失败次数（fail-open 落空）。
    pub alert_failed: u64,
    /// 最近一轮流水线的各 Stage 耗时（ms，未跑过为 None）。
    pub last_capture_ms: Option<f64>,
    pub last_layout_ms: Option<f64>,
    pub last_ocr_ms: Option<f64>,
    pub last_sem_ms: Option<f64>,
}

/// models/ 下的有效模型（`fs://models` 载荷；model.toml 非法的目录由 ModelStore 跳过）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelDto {
    /// 目录名（配置 `layout.model` 引用的值）。
    pub name: String,
    pub kind: String,
    pub version: String,
    pub note: String,
    /// 来源：`builtin`（内置，exe 同目录 resources/models/）或 `user`（数据目录 models/）。
    pub source: String,
}

/// datasets/ 下的训练数据 zip（`fs://datasets` 载荷；uitag 导出物）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetDto {
    /// zip 文件名（含扩展名）。
    pub name: String,
    pub size_bytes: u64,
}

/// 训练任务状态（`training://status` 载荷）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrainingStatusDto {
    /// idle | running | success | error
    pub state: String,
    /// 本次训练的数据集名。
    pub dataset: String,
    /// 产出的模型目录名（success 时）。
    pub model: Option<String>,
    pub message: Option<String>,
}

impl Default for TrainingStatusDto {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            dataset: String::new(),
            model: None,
            message: None,
        }
    }
}

fn level_str(l: VerdictLevel) -> &'static str {
    match l {
        VerdictLevel::Safe => "safe",
        VerdictLevel::Warn => "warn",
        VerdictLevel::Block => "block",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UT-BRG-01 伴随：DTO 字段齐全、draft_text 进入弹窗载荷。
    #[test]
    fn alert_payload_mapping() {
        let v = Verdict::block("命中违禁词「sb」(severity=block)")
            .with_epoch(3)
            .with_draft("你是 sb", 42)
            .with_target("张总")
            .with_scene("正式")
            .with_context(Some("之前聊天内容".to_string()));
        let p = AlertPayload::from_verdict(&v, 10);
        assert_eq!(p.level, "block");
        assert_eq!(p.draft_text, "你是 sb");
        assert_eq!(p.draft_fingerprint, 42);
        assert_eq!(p.draft_epoch, 3);
        assert_eq!(p.chat_target.as_deref(), Some("张总"));
        assert_eq!(p.scene_name.as_deref(), Some("正式"));
        assert_eq!(p.chat_context.as_deref(), Some("之前聊天内容"));
        assert_eq!(p.countdown_secs, 10);
        assert!(p.reasons[0].contains("sb"));
    }
}
