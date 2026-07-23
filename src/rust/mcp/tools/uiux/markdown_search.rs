// UI/UX markdown 资料本地检索
// 用于在 fast-context 不可用时，直接搜索内置的 ui-ux-pro-max-skill.md。
// 检索策略：按标题结构分块 + 中文 bigram 分词 + 命中率归一化打分，
// 解决旧实现"整段中文当单 token 导致 contains 永远无法命中"的问题。

use std::collections::HashSet;

use serde::Serialize;

use crate::mcp::tools::memory::TextSimilarity;

const UIUX_MARKDOWN_PATH: &str = "src/rust/assets/resources/ui-ux-pro-max-skill.md";
const UIUX_MARKDOWN: &str = include_str!("../../../assets/resources/ui-ux-pro-max-skill.md");
// 超长小节的细分窗口：知识库多为"一行一条"的列表/表格，24 行足够容纳一组条目
const SECTION_MAX_LINES: usize = 24;
const SECTION_OVERLAP: usize = 4;
const MAX_EXCERPT_CHARS: usize = 900;

#[derive(Debug, Clone, Serialize)]
pub struct MarkdownHit {
    pub source: String,
    pub location: String,
    pub excerpt: String,
}

#[derive(Debug, Clone)]
struct MarkdownChunk {
    heading: String,
    start_line: usize,
    end_line: usize,
    content: String,
}

pub fn search_markdown(query: &str, max_results: usize) -> Vec<MarkdownHit> {
    let limit = max_results.max(1);
    // 分词只做一次，避免每个 chunk 重复计算
    let tokens = collect_query_tokens(query);
    let mut ranked: Vec<(f64, MarkdownHit)> = chunk_markdown(UIUX_MARKDOWN)
        .into_iter()
        .filter(is_searchable_chunk)
        .filter_map(|chunk| {
            let score = score_chunk(query, &tokens, &chunk);
            if score <= 0.0 {
                return None;
            }

            let excerpt = build_excerpt(&chunk);
            let hit = MarkdownHit {
                source: "local_markdown".to_string(),
                location: format!(
                    "{}:{}-{}",
                    UIUX_MARKDOWN_PATH, chunk.start_line, chunk.end_line
                ),
                excerpt,
            };
            Some((score, hit))
        })
        .collect();

    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked.into_iter().map(|(_, hit)| hit).take(limit).collect()
}

fn is_searchable_chunk(chunk: &MarkdownChunk) -> bool {
    !matches!(
        chunk.heading.as_str(),
        "使用说明" | "当前推荐调用方式" | "可用技术栈"
    )
}

/// 按标题结构分块：一节一块保持语义完整（表格/列表不再被盲切拦腰截断）；
/// 超长小节按固定窗口细分并携带所属标题作为上下文。
fn chunk_markdown(text: &str) -> Vec<MarkdownChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    // 第一步：按标题行切出章节区间（0 基闭区间）
    let mut sections: Vec<(String, usize, usize)> = Vec::new();
    let mut current_heading = "UI/UX Pro Max".to_string();
    let mut section_start = 0usize;
    for (index, line) in lines.iter().enumerate() {
        if let Some(heading) = parse_heading(line) {
            if index > section_start {
                sections.push((current_heading.clone(), section_start, index - 1));
            }
            current_heading = heading;
            section_start = index;
        }
    }
    if section_start < lines.len() {
        sections.push((current_heading, section_start, lines.len() - 1));
    }

    // 第二步：超长章节细分为带重叠的窗口
    let mut chunks = Vec::new();
    for (heading, start, end) in sections {
        let mut cursor = start;
        loop {
            let window_end = (cursor + SECTION_MAX_LINES - 1).min(end);
            let content = lines[cursor..=window_end].join("\n");
            if !content.trim().is_empty() {
                chunks.push(MarkdownChunk {
                    heading: heading.clone(),
                    start_line: cursor + 1,
                    end_line: window_end + 1,
                    content,
                });
            }
            if window_end >= end {
                break;
            }
            // 保证游标严格前进，避免重叠参数变化引入死循环
            cursor = (window_end + 1)
                .saturating_sub(SECTION_OVERLAP)
                .max(cursor + 1);
        }
    }

    chunks
}

fn parse_heading(line: &str) -> Option<String> {
    line.strip_prefix("### ")
        .or_else(|| line.strip_prefix("## "))
        .or_else(|| line.strip_prefix("# "))
        .map(|heading| heading.trim().to_string())
}

/// 打分模型：整句包含（强信号）+ 内容命中率*3 + 标题命中率*1.5 + 字符相似度（弱补充）。
/// 采用命中率归一化而非绝对计数，避免长查询/长块场景下的天然偏置。
fn score_chunk(query: &str, tokens: &[String], chunk: &MarkdownChunk) -> f64 {
    let normalized_query = query.trim().to_lowercase();
    let normalized_content = chunk.content.to_lowercase();
    let normalized_heading = chunk.heading.to_lowercase();
    if normalized_query.is_empty() || normalized_content.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    if normalized_content.contains(&normalized_query) {
        score += 4.0;
    }

    let mut matched_tokens = 0usize;
    let mut matched_heading_tokens = 0usize;
    for token in tokens {
        if normalized_content.contains(token.as_str()) {
            matched_tokens += 1;
        }
        if normalized_heading.contains(token.as_str()) {
            matched_heading_tokens += 1;
        }
    }
    if !tokens.is_empty() {
        score += matched_tokens as f64 / tokens.len() as f64 * 3.0;
        // 标题命中说明整节主题相关，额外加权
        score += matched_heading_tokens as f64 / tokens.len() as f64 * 1.5;
    }

    // 字符级相似度权重调低为 1.0（旧为 2.0），避免页脚/品牌字样等噪声干扰排序
    score += TextSimilarity::calculate_enhanced(&normalized_query, &normalized_content);

    // 完全无 token 命中且弱信号不足时视为不相关
    if matched_tokens == 0 && matched_heading_tokens == 0 && score < 0.6 {
        0.0
    } else {
        score
    }
}

fn build_excerpt(chunk: &MarkdownChunk) -> String {
    let mut out = String::new();
    out.push_str(&format!("标题：{}\n", chunk.heading));
    out.push_str(chunk.content.trim());
    truncate_text(&out, MAX_EXCERPT_CHARS)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }

    let truncated: String = text.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

/// 查询分词：ASCII 连续段按词切分（统一小写）；连续中文段切成 bigram（2-gram），
/// 使"五档难度参数"能以"五档/档难/难度/度参/参数"匹配知识库行文。
/// 结果按出现顺序去重，避免重复词干扰命中率分母。
fn collect_query_tokens(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut ascii_buffer = String::new();
    let mut cjk_buffer: Vec<char> = Vec::new();

    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() {
            flush_cjk_tokens(&mut tokens, &mut cjk_buffer);
            ascii_buffer.push(ch.to_ascii_lowercase());
        } else if is_cjk(ch) {
            flush_ascii_token(&mut tokens, &mut ascii_buffer);
            cjk_buffer.push(ch);
        } else {
            flush_ascii_token(&mut tokens, &mut ascii_buffer);
            flush_cjk_tokens(&mut tokens, &mut cjk_buffer);
        }
    }
    flush_ascii_token(&mut tokens, &mut ascii_buffer);
    flush_cjk_tokens(&mut tokens, &mut cjk_buffer);

    let mut seen = HashSet::new();
    tokens.retain(|token| seen.insert(token.clone()));
    tokens
}

fn is_cjk(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
}

fn flush_ascii_token(tokens: &mut Vec<String>, buffer: &mut String) {
    // 单字符 ASCII（如量词、序号）噪声大，忽略
    if buffer.chars().count() >= 2 {
        tokens.push(buffer.clone());
    }
    buffer.clear();
}

fn flush_cjk_tokens(tokens: &mut Vec<String>, buffer: &mut Vec<char>) {
    // 单个汉字噪声大，忽略；两字及以上切 bigram
    if buffer.len() == 2 {
        tokens.push(buffer.iter().collect());
    } else if buffer.len() > 2 {
        for pair in buffer.windows(2) {
            tokens.push(pair.iter().collect());
        }
    }
    buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_query_produces_bigram_tokens() {
        let tokens = collect_query_tokens("五档难度参数和主题视觉 Stroop");
        // 连续中文应被切成 bigram，而非整段单 token
        assert!(tokens.contains(&"五档".to_string()));
        assert!(tokens.contains(&"难度".to_string()));
        assert!(tokens.contains(&"参数".to_string()));
        assert!(tokens.contains(&"主题".to_string()));
        // ASCII 词统一小写
        assert!(tokens.contains(&"stroop".to_string()));
        // 不应出现旧实现的整段 token
        assert!(!tokens.contains(&"五档难度参数和主题视觉".to_string()));
    }

    #[test]
    fn tokens_are_deduplicated() {
        let tokens = collect_query_tokens("配色 配色 color color");
        assert_eq!(
            tokens.iter().filter(|t| t.as_str() == "配色").count(),
            1,
            "重复中文词应去重"
        );
        assert_eq!(
            tokens.iter().filter(|t| t.as_str() == "color").count(),
            1,
            "重复 ASCII 词应去重"
        );
    }

    #[test]
    fn chinese_query_matches_relevant_sections() {
        // 旧实现下该类纯中文查询因整段 token 无法命中而大量返回空
        let hits = search_markdown("仪表盘 配色 色彩方案 字体配对", 3);
        assert!(!hits.is_empty(), "中文查询应能命中知识库相关小节");
    }

    #[test]
    fn brand_footer_is_not_top_hit_for_topic_query() {
        // 复现用户反馈场景：主题类查询不应命中文档页脚的"可用技术栈"表
        let hits = search_markdown("专注力测试 难度参数 主题视觉 计时压力反馈 结果可读性", 3);
        if let Some(first) = hits.first() {
            assert!(
                !first.excerpt.contains("本文档由 UI/UX Pro Max 数据库生成"),
                "页脚生成声明不应成为首位命中: {}",
                first.excerpt
            );
        }
    }

    #[test]
    fn chunks_follow_heading_structure() {
        let chunks = chunk_markdown("# 总标题\n内容A\n## 小节一\n内容B\n内容C\n## 小节二\n内容D");
        assert_eq!(chunks.len(), 3, "应按标题切成 3 块");
        assert_eq!(chunks[1].heading, "小节一");
        assert_eq!(chunks[1].start_line, 3);
        assert_eq!(chunks[1].end_line, 5);
        assert_eq!(chunks[2].heading, "小节二");
    }

    #[test]
    fn representative_queries_keep_relevant_knowledge_in_top_three() {
        let cases = [
            ("金融仪表盘 收入 现金流 审计", "Data-Dense Dashboard"),
            ("表单 错误位置 内联验证 提交反馈", "表单"),
            ("趋势 时间序列 折线图", "折线图"),
            ("移动端 触摸目标 44x44", "触摸目标"),
            ("glassmorphism 毛玻璃 模态框", "Glassmorphism"),
            ("字体配对 仪表盘 数据 精确", "Dashboard Data"),
        ];

        for (query, expected) in cases {
            let hits = search_markdown(query, 3);
            assert!(
                hits.iter().any(|hit| hit.excerpt.contains(expected)),
                "查询 {query:?} 的 top-3 应包含 {expected:?}，实际为: {hits:?}"
            );
        }
    }

    #[test]
    fn usage_metadata_is_excluded_from_knowledge_hits() {
        let hits = search_markdown("glassmorphism 金融仪表盘 页面美化", 8);
        assert!(hits.iter().all(|hit| {
            !hit.excerpt.contains("当前推荐调用方式") && !hit.excerpt.contains("### 可用技术栈")
        }));
    }
}
