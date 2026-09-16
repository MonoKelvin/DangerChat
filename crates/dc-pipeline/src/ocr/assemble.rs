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
/// - **短行（≤2 字符）一律丢弃**：输入框截图里表情/语音按钮边缘会被 det 切出
///   「%」「白」「)」这类碎片行；真实草稿通常更长（单字回复如「好」的损失
///   由 M5 语义层空草稿 Safe 语义兜底，属可接受取舍）。
pub fn filter_lines(lines: Vec<RawLine>, noise_words: &[String], min_conf: f32) -> Vec<RawLine> {
    lines
        .into_iter()
        .filter(|l| l.score >= min_conf)
        .filter(|l| !l.text.trim().is_empty())
        .filter(|l| l.text.trim().chars().count() > 2)
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

    /// UT-OCR-04：噪声词（发送按钮）与 UI 碎片短行被过滤。
    #[test]
    fn ut_ocr_04_noise_filter() {
        let noise = vec!["发送".to_string(), "按住鼠标 语音输入文字".to_string()];
        let lines = vec![
            line("你好世界", 0.9, 10, 10),
            line("发送", 0.99, 500, 600),                  // 精确噪声词
            line("按住鼠标 语音输入文字", 0.99, 500, 630), // 精确噪声词（含空格）
            line("发", 0.9, 10, 40),                       // 短行 → 过滤
            line("%", 0.89, 20, 40),                       // UI 碎片短行（高分也滤）→ 过滤
            line("白(", 0.8, 20, 60),                      // 2 字符碎片 → 过滤
            line("三个字", 0.9, 10, 70),                   // 3 字符 → 保留
        ];
        let r = assemble(lines, &noise, 0.5);
        assert_eq!(r.draft_text, "你好世界\n三个字");
        assert_eq!(r.blocks.len(), 2);
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
}
