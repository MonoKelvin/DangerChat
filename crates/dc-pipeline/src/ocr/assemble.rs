//! OCR 结果组装（§5.6）：分行拼接、噪声过滤、字段提取。
//!
//! 输入是 rapidocr 的 `OcrLine` 语义等价物（text/score/bbox），输出契约 `OcrResult`。

use crate::contract::{OcrResult, TextBlock};
use dc_sys::Rect;

/// 一行 OCR 原始输出（engine 产出，组装输入）。
#[derive(Debug, Clone, PartialEq)]
pub struct RawLine {
    pub text: String,
    pub score: f32,
    /// 行包围盒（ROI 局部坐标）。
    pub rect: Rect,
}

/// 按行拼接草稿文本：y 中心排序（自上而下），行内已由 det 保证 x 序。
/// 每行末尾补换行（最后一段不加——「按行拼接」的语义是保序而非保显式换行）。
pub fn join_draft(lines: &[RawLine]) -> String {
    let mut sorted: Vec<&RawLine> = lines.iter().collect();
    sorted.sort_by(|a, b| {
        let (ay, by) = (
            a.rect.y + a.rect.h as i32 / 2,
            b.rect.y + b.rect.h as i32 / 2,
        );
        ay.cmp(&by).then(a.rect.x.cmp(&b.rect.x))
    });
    sorted
        .iter()
        .map(|l| l.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 过滤规则集合（§5.6）：
/// - 置信度 < `min_conf` 丢弃；
/// - 整行等于噪声词（发送按钮一类）丢弃；
/// - **保留所有非空行**：不再以字符长度过滤，让 L1 规则覆盖短敏感词
///   （如「sb」「傻逼」）；UI 碎片（表情/语音按钮边缘切出的「%」「白」「)」等）
///   由噪声词表负责，而非长度阈值。
pub fn filter_lines(lines: Vec<RawLine>, noise_words: &[String], min_conf: f32) -> Vec<RawLine> {
    lines
        .into_iter()
        .filter(|l| l.score >= min_conf)
        .filter(|l| !l.text.trim().is_empty())
        // 保留所有非空行（空行已过滤），不再按字符数过滤——短敏感词如「sb」「傻逼」
        // 必须进入草稿交由 L1 规则判定；UI 碎片由 noise_words 过滤。
        .filter(|l| {
            let t = l.text.trim();
            !noise_words.iter().any(|w| w == t)
        })
        .collect()
}

/// 组装最终 OcrResult。
///
/// - `draft_text`：msg_input 行拼接（过滤后）
/// - `chat_target`：chat_target 区域置信度最高的行（慢环才有；快环传 None 由调用方缓存）
pub fn assemble(blocks: Vec<RawLine>, noise_words: &[String], min_conf: f32) -> OcrResult {
    let filtered = filter_lines(blocks, noise_words, min_conf);
    let draft_text = join_draft(&filtered);
    let blocks = filtered
        .into_iter()
        .map(|l| TextBlock {
            text: l.text,
            rect: l.rect,
            confidence: l.score,
        })
        .collect();
    OcrResult {
        chat_target: None,
        draft_text,
        blocks,
        chat_context: None,
    }
}

/// 从 chat_target 区域行中取置信度最高的一行作为对象名。
pub fn pick_chat_target(lines: &[RawLine], min_conf: f32) -> Option<String> {
    lines
        .iter()
        .filter(|l| l.score >= min_conf && !l.text.trim().is_empty())
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|l| l.text.trim().to_string())
}

/// 从 chat_window 区域的 OCR 行中提取最近对话摘要。
///
/// 聊天窗口 OCR 输出通常是交错的多条消息（每条消息一行或多行）。取最后 N 条
/// 非空行拼接作为上下文，供 L2 语义判定消歧（§5.7-context）。N=5 恰好在
/// BGE 嵌入输入长度预算内（~64 token），避免超长输入拖慢快环。
///
/// 行序来自 OCR 引擎，按 y 中心排序（自上而下），因此 last N 是视觉上
/// 最近的 N 条文本。
pub fn extract_chat_context(lines: &[RawLine], min_conf: f32, max_lines: usize) -> Option<String> {
    let mut sorted: Vec<&RawLine> = lines.iter().collect();
    sorted.sort_by(|a, b| {
        let (ay, by) = (
            a.rect.y + a.rect.h as i32 / 2,
            b.rect.y + b.rect.h as i32 / 2,
        );
        ay.cmp(&by).then(a.rect.x.cmp(&b.rect.x))
    });
    let recent: Vec<&str> = sorted
        .iter()
        .rev()
        .filter(|l| l.score >= min_conf)
        .map(|l| l.text.trim())
        .filter(|t| !t.is_empty())
        .take(max_lines)
        .collect();
    if recent.is_empty() {
        None
    } else {
        Some(recent.into_iter().rev().collect::<Vec<_>>().join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, score: f32, x: i32, y: i32) -> RawLine {
        RawLine {
            text: text.into(),
            score,
            rect: Rect::new(x, y, 50, 20),
        }
    }

    /// UT-OCR-02：空输入框 → draft_text 为空。
    #[test]
    fn ut_ocr_02_empty_input() {
        let r = assemble(vec![], &[], 0.5);
        assert_eq!(r.draft_text, "");
        assert!(r.blocks.is_empty());

        // 只有噪声/低分行也视为空
        let r = assemble(
            vec![line("发送", 0.9, 0, 0), line("x", 0.1, 0, 30)],
            &["发送".to_string()],
            0.5,
        );
        assert_eq!(r.draft_text, "");
    }

    /// UT-OCR-03：多行按 y 序拼接（乱序输入 → 正确顺序）。
    #[test]
    fn ut_ocr_03_multiline_order() {
        let lines = vec![
            line("第二行", 0.9, 10, 50),
            line("第一行", 0.9, 10, 10),
            line("第三行", 0.9, 10, 90),
        ];
        let r = assemble(lines, &[], 0.5);
        assert_eq!(r.draft_text, "第一行\n第二行\n第三行");
    }

    /// UT-OCR-04：噪声词（发送按钮）被过滤；短行保留（由噪声词表而非长度过滤）。
    #[test]
    fn ut_ocr_04_noise_filter() {
        let noise = vec!["发送".to_string(), "按住鼠标 语音输入文字".to_string()];
        let lines = vec![
            line("你好世界", 0.9, 10, 10),
            line("发送", 0.99, 500, 600), // 精确噪声词 → 过滤
            line("按住鼠标 语音输入文字", 0.99, 500, 630), // 精确噪声词（含空格） → 过滤
            line("sb", 0.9, 10, 40),      // 短敏感词 → 保留（L1 规则判定）
            line("三个字", 0.9, 10, 70),  // 3 字符 → 保留
        ];
        let r = assemble(lines, &noise, 0.5);
        assert_eq!(r.draft_text, "你好世界\nsb\n三个字");
        assert_eq!(r.blocks.len(), 3);
    }

    /// min_conf 边界：恰好等于阈值保留。
    #[test]
    fn min_conf_boundary() {
        let lines = vec![line("上边界", 0.5, 0, 0), line("下边界", 0.4999, 0, 30)];
        let r = assemble(lines, &[], 0.5);
        assert_eq!(r.draft_text, "上边界");
    }

    #[test]
    fn chat_target_picks_best_confidence() {
        let lines = vec![
            line("文件传输助手", 0.82, 0, 0),
            line("张总", 0.95, 0, 30),
            line("低分", 0.3, 0, 60),
        ];
        assert_eq!(pick_chat_target(&lines, 0.5), Some("张总".to_string()));
        assert_eq!(pick_chat_target(&[], 0.5), None);
    }

    /// UT-OCR-05：短敏感词（≤2 字符）不被过滤，进入草稿以便 L1 判定。
    /// 回归测试：曾因 `chars().count() > 2` 过滤导致「sb」「傻逼」等短敏感词被
    /// 丢弃 → 空草稿 → Safe（未拦截）。
    #[test]
    fn ut_ocr_05_short_sensitive_word_preserved() {
        let lines = vec![
            line("sb", 0.9, 10, 10),    // 2 字符
            line("傻逼", 0.85, 10, 30), // 2 字符
        ];
        let r = assemble(lines, &[], 0.5);
        assert_eq!(r.draft_text, "sb\n傻逼");
        assert_eq!(r.blocks.len(), 2);
    }

    /// UT-OCR-06-context：从多行聊天窗口 OCR 中提取最近 N 条作为上下文。
    #[test]
    fn ut_ocr_06_context_extraction() {
        let lines = vec![
            line("早上好", 0.9, 10, 10),
            line("昨天的报告怎么样", 0.88, 10, 50),
            line("已经完成了", 0.92, 10, 90),
            line("不错", 0.85, 10, 130),
            line("今天开会讨论吧", 0.91, 10, 170),
            line("低分行", 0.3, 10, 210), // 低置信 → 过滤
        ];
        // max_lines=3 → 取最后 3 个有效行
        let ctx = extract_chat_context(&lines, 0.5, 3);
        assert_eq!(ctx, Some("已经完成了\n不错\n今天开会讨论吧".to_string()));
    }

    /// UT-OCR-07-context：空输入 / 全低分 → None。
    #[test]
    fn ut_ocr_07_context_empty() {
        assert_eq!(extract_chat_context(&[], 0.5, 5), None);
        let lines = vec![line("低分", 0.2, 0, 0), line("噪声", 0.1, 0, 30)];
        assert_eq!(extract_chat_context(&lines, 0.5, 5), None);
    }

    /// UT-OCR-08-context：max_lines 限长 —— 超过 N 行时只取最近 N。
    #[test]
    fn ut_ocr_08_context_max_lines() {
        let lines: Vec<_> = (0..10)
            .map(|i| line(&format!("消息{i}"), 0.9, 0, i * 20))
            .collect();
        let ctx = extract_chat_context(&lines, 0.5, 3);
        assert_eq!(ctx, Some("消息7\n消息8\n消息9".to_string()));
    }

    /// UT-OCR-09-context：噪声词过滤后再提取上下文（模拟 OCR stage 的 filter_lines + extract）。
    #[test]
    fn ut_ocr_09_context_with_noise_filter() {
        let noise = vec!["发送".to_string()];
        let lines = vec![
            line("你好", 0.9, 10, 10),
            line("发送", 0.9, 10, 50), // 噪声 → 过滤
            line("收到了", 0.88, 10, 90),
            line("不错", 0.91, 10, 130),
        ];
        let filtered = filter_lines(lines, &noise, 0.5);
        let ctx = extract_chat_context(&filtered, 0.5, 3);
        assert_eq!(ctx, Some("你好\n收到了\n不错".to_string()));
    }
}
