// UI/UX markdown 资料本地检索
// 用于在 sou 不可用时，直接搜索内置的 ui-ux-pro-max-skill.md

use serde::Serialize;

use crate::mcp::tools::memory::TextSimilarity;

const UIUX_MARKDOWN_PATH: &str = "src/rust/assets/resources/ui-ux-pro-max-skill.md";
const UIUX_MARKDOWN: &str = include_str!("../../../assets/resources/ui-ux-pro-max-skill.md");
const CHUNK_LINES: usize = 18;
const CHUNK_OVERLAP: usize = 4;
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

pub fn source_path() -> &'static str {
    UIUX_MARKDOWN_PATH
}

pub fn search_markdown(query: &str, max_results: usize) -> Vec<MarkdownHit> {
    let limit = max_results.max(1) as usize;
    let mut ranked: Vec<(f64, MarkdownHit)> = chunk_markdown(UIUX_MARKDOWN)
        .into_iter()
        .filter_map(|chunk| {
            let score = score_chunk(query, &chunk.content);
            if score <= 0.0 {
                return None;
            }

            let excerpt = build_excerpt(&chunk);
            let hit = MarkdownHit {
                source: "local_markdown".to_string(),
                location: format!("{}:{}-{}", UIUX_MARKDOWN_PATH, chunk.start_line, chunk.end_line),
                excerpt,
            };
            Some((score, hit))
        })
        .collect();

    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked
        .into_iter()
        .map(|(_, hit)| hit)
        .take(limit)
        .collect()
}

fn chunk_markdown(text: &str) -> Vec<MarkdownChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current_heading = "UI/UX Pro Max".to_string();
    let mut index = 0usize;

    while index < lines.len() {
        let mut window = Vec::new();
        let start_line = index + 1;
        let mut consumed = 0usize;
        let mut local_heading = current_heading.clone();

        while index + consumed < lines.len() && consumed < CHUNK_LINES {
            let line = lines[index + consumed];
            if let Some(heading) = line.strip_prefix("### ") {
                local_heading = heading.trim().to_string();
                current_heading = local_heading.clone();
            } else if let Some(heading) = line.strip_prefix("## ") {
                local_heading = heading.trim().to_string();
                current_heading = local_heading.clone();
            } else if let Some(heading) = line.strip_prefix("# ") {
                local_heading = heading.trim().to_string();
                current_heading = local_heading.clone();
            }

            window.push(line);
            consumed += 1;
        }

        let end_line = start_line + consumed.saturating_sub(1);
        let content = window.join("\n");
        if !content.trim().is_empty() {
            chunks.push(MarkdownChunk {
                heading: local_heading,
                start_line,
                end_line,
                content,
            });
        }

        if consumed <= CHUNK_OVERLAP {
            index += consumed.max(1);
        } else {
            index += consumed - CHUNK_OVERLAP;
        }
    }

    chunks
}

fn score_chunk(query: &str, chunk: &str) -> f64 {
    let normalized_query = query.trim().to_lowercase();
    let normalized_chunk = chunk.to_lowercase();
    if normalized_query.is_empty() || normalized_chunk.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    if normalized_chunk.contains(&normalized_query) {
        score += 4.0;
    }

    let mut matched_tokens = 0usize;
    for token in collect_query_tokens(query) {
        if token.len() < 2 {
            continue;
        }
        if normalized_chunk.contains(&token.to_lowercase()) {
            matched_tokens += 1;
        }
    }
    score += matched_tokens as f64;
    score += TextSimilarity::calculate_enhanced(&normalized_query, &normalized_chunk) * 2.0;

    if matched_tokens == 0 && score < 0.8 {
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

fn collect_query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    let mut buffer_is_ascii = None;

    for ch in query.chars() {
        let is_ascii_word = ch.is_ascii_alphanumeric();
        let is_cjk = ('\u{4E00}'..='\u{9FFF}').contains(&ch);

        if !is_ascii_word && !is_cjk {
            flush_token(&mut tokens, &mut buffer);
            buffer_is_ascii = None;
            continue;
        }

        match buffer_is_ascii {
            Some(flag) if flag == is_ascii_word => buffer.push(ch),
            Some(_) => {
                flush_token(&mut tokens, &mut buffer);
                buffer.push(ch);
                buffer_is_ascii = Some(is_ascii_word);
            }
            None => {
                buffer.push(ch);
                buffer_is_ascii = Some(is_ascii_word);
            }
        }
    }

    flush_token(&mut tokens, &mut buffer);
    tokens
}

fn flush_token(tokens: &mut Vec<String>, buffer: &mut String) {
    let token = buffer.trim();
    if token.len() >= 2 {
        tokens.push(token.to_string());
    }
    buffer.clear();
}
