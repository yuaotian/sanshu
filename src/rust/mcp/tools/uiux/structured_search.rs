//! UI/UX Pro Max v2.15.0 本地结构化检索。
//!
//! 该实现按上游 CSV 数据域分别建立 BM25 索引，并在查询侧做少量中英文概念改写。
//! 检索结果经过低置信度拒答与跨域去重，避免长叙事查询被通用词稀释。

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;

use super::lexicon;
use super::types::UiuxAction;

pub const KNOWLEDGE_VERSION: &str = "v2.15.0";
pub const KNOWLEDGE_ENGINE: &str = "structured_bm25_v1";

const ASSET_ROOT: &str = "src/rust/assets/resources/ui-ux-pro-max-v2.15.0";
const MAX_EXCERPT_CHARS: usize = 1_100;
const BM25_K1: f64 = 1.5;
const BM25_B: f64 = 0.75;

const STYLE_SEARCH: &[&str] = &[
    "Style ID",
    "Style Category",
    "Aliases",
    "Keywords",
    "Best For",
    "Type",
    "AI Prompt Keywords",
    "CSS/Technical Keywords",
    "Performance",
    "Effects & Animation",
];
const STYLE_DISPLAY: &[&str] = &[
    "Style Category",
    "Keywords",
    "Primary Colors",
    "Effects & Animation",
    "Best For",
    "Do Not Use For",
    "Performance",
    "Accessibility",
    "Complexity",
    "AI Prompt Keywords",
    "CSS/Technical Keywords",
];
const PRODUCT_SEARCH: &[&str] = &[
    "Product Type",
    "Keywords",
    "Primary Style Recommendation",
    "Secondary Styles",
    "Key Considerations",
];
const PRODUCT_DISPLAY: &[&str] = &[
    "Product Type",
    "Keywords",
    "Primary Style Recommendation",
    "Secondary Styles",
    "Landing Page Pattern",
    "Dashboard Style (if applicable)",
    "Color Palette Focus",
    "Key Considerations",
];
const MOTION_SEARCH: &[&str] = &[
    "Category",
    "Intensity Tier",
    "Keywords",
    "Trigger",
    "Performance Notes",
];
const MOTION_DISPLAY: &[&str] = &[
    "Category",
    "Intensity Tier",
    "Trigger",
    "Duration",
    "Easing",
    "Do",
    "Don't",
    "Performance Notes",
];
const GUIDELINE_SEARCH: &[&str] = &["Category", "Issue", "Keywords", "Description", "Platform"];
const GUIDELINE_DISPLAY: &[&str] = &[
    "Category",
    "Issue",
    "Platform",
    "Description",
    "Do",
    "Don't",
    "Severity",
];
const STACK_SEARCH: &[&str] = &[
    "Category",
    "Guideline",
    "Description",
    "Do",
    "Don't",
    "Applies To",
];
const STACK_DISPLAY: &[&str] = &[
    "Category",
    "Guideline",
    "Description",
    "Do",
    "Don't",
    "Severity",
    "Applies To",
    "Status",
    "Verified At",
];

#[derive(Debug, Clone)]
pub struct KnowledgeHit {
    pub source: String,
    pub location: String,
    pub excerpt: String,
    pub domain: String,
}

/// 提供给本地语义索引的稳定文档视图；顺序与内置 CSV 语料顺序一致。
#[derive(Debug, Clone)]
pub struct SemanticDocument {
    pub identity: String,
    pub domain: String,
    pub location: String,
    pub text: String,
    pub excerpt: String,
}

#[derive(Debug, Clone)]
pub struct SearchReport {
    pub hits: Vec<KnowledgeHit>,
    pub rewritten_query: String,
    pub query_rewrites: Vec<String>,
    pub domains: Vec<String>,
    pub top_score: f64,
    pub token_coverage: f64,
    pub abstained: bool,
}

#[derive(Clone, Copy)]
struct CorpusSpec {
    domain: &'static str,
    label: &'static str,
    relative_path: &'static str,
    csv: &'static str,
    identity_fields: &'static [&'static str],
    search_fields: &'static [&'static str],
    display_fields: &'static [&'static str],
    exclude_deprecated_styles: bool,
}

#[derive(Debug)]
struct KnowledgeDocument {
    identity: String,
    normalized_identity: String,
    term_frequencies: HashMap<String, usize>,
    length: usize,
    search_text: String,
    location: String,
    excerpt: String,
}

#[derive(Debug)]
struct DomainIndex {
    domain: String,
    label: String,
    documents: Vec<KnowledgeDocument>,
    document_frequencies: HashMap<String, usize>,
    average_length: f64,
}

#[derive(Debug)]
struct Candidate {
    identity: String,
    domain: String,
    hit: KnowledgeHit,
    score: f64,
    raw_score: f64,
    coverage: f64,
}

static INDEXES: Lazy<Vec<DomainIndex>> = Lazy::new(|| {
    corpus_specs()
        .into_iter()
        .filter_map(build_domain_index)
        .collect()
});

static MATERIALIZED_MARKDOWN: Lazy<String> = Lazy::new(|| {
    let mut output = format!(
        "# UI/UX Pro Max {KNOWLEDGE_VERSION}\n\n来源：nextlevelbuilder/ui-ux-pro-max-skill，检索引擎：{KNOWLEDGE_ENGINE}。\n"
    );
    for index in INDEXES.iter() {
        output.push_str(&format!("\n## {} ({})\n", index.label, index.domain));
        for document in &index.documents {
            output.push_str(&format!(
                "\n### {}\n来源位置：{}\n{}\n",
                document.identity, document.location, document.excerpt
            ));
        }
    }
    output
});

static SEMANTIC_DOCUMENTS: Lazy<Vec<SemanticDocument>> = Lazy::new(|| {
    INDEXES
        .iter()
        .flat_map(|index| {
            index.documents.iter().map(|document| SemanticDocument {
                identity: document.identity.clone(),
                domain: index.domain.clone(),
                location: document.location.clone(),
                text: format!(
                    "{}\n{}\n{}",
                    document.identity, document.search_text, document.excerpt
                ),
                excerpt: document.excerpt.clone(),
            })
        })
        .collect()
});

pub fn search(query: &str, action: UiuxAction, max_results: usize) -> SearchReport {
    let limit = max_results.clamp(1, 8);
    let rewritten = rewrite_query(query);
    if rewritten.tokens.is_empty() {
        return empty_report(rewritten);
    }
    let raw_query_tokens = tokenize(query);

    let routing = route_domains(query, &rewritten.tokens, action);
    let minimum_matches = if rewritten.tokens.len() <= 3 { 1 } else { 2 };
    let mut candidates = Vec::new();

    for index in INDEXES.iter() {
        if !routing.domains.contains(index.domain.as_str()) {
            continue;
        }

        let searchable_tokens: Vec<&String> = rewritten
            .tokens
            .iter()
            .filter(|token| index.document_frequencies.contains_key(token.as_str()))
            .collect();
        if searchable_tokens.is_empty() {
            continue;
        }

        for document in &index.documents {
            let matched = searchable_tokens
                .iter()
                .filter(|token| document.term_frequencies.contains_key(token.as_str()))
                .count();
            // 身份字段加权只认用户原文，查询扩展词不能伪装成显式 Style ID/产品名。
            let identity_bonus = identity_bonus(query, &raw_query_tokens, document);
            if matched < minimum_matches && identity_bonus == 0.0 {
                continue;
            }

            let raw_score = bm25_score(index, document, &searchable_tokens);
            if raw_score <= 0.0 {
                continue;
            }

            let coverage = matched as f64 / searchable_tokens.len() as f64;
            let route_boost = routing
                .strong_domains
                .contains(index.domain.as_str())
                .then_some(0.55)
                .unwrap_or(0.15);
            let normalized_bm25 = raw_score / (raw_score + 5.0);
            let score = normalized_bm25 * 4.0
                + coverage * 2.0
                + (matched as f64).ln_1p() * 0.35
                + route_boost
                + identity_bonus;

            candidates.push(Candidate {
                identity: document.normalized_identity.clone(),
                domain: index.domain.clone(),
                hit: KnowledgeHit {
                    source: "local_bm25".to_string(),
                    location: document.location.clone(),
                    excerpt: document.excerpt.clone(),
                    domain: index.domain.clone(),
                },
                score,
                raw_score,
                coverage,
            });
        }
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.hit.location.cmp(&right.hit.location))
    });
    let top_score = candidates.first().map(|item| item.raw_score).unwrap_or(0.0);
    let token_coverage = candidates.first().map(|item| item.coverage).unwrap_or(0.0);
    let hits = select_diverse_hits(candidates, limit);
    let domains = ordered_unique(hits.iter().map(|hit| hit.domain.clone()));

    SearchReport {
        abstained: hits.is_empty(),
        hits,
        rewritten_query: rewritten.text,
        query_rewrites: rewritten.rewrites,
        domains,
        top_score,
        token_coverage,
    }
}

pub fn materialized_markdown() -> &'static str {
    MATERIALIZED_MARKDOWN.as_str()
}

pub fn semantic_documents() -> &'static [SemanticDocument] {
    SEMANTIC_DOCUMENTS.as_slice()
}

#[cfg(test)]
fn document_count() -> usize {
    INDEXES.iter().map(|index| index.documents.len()).sum()
}

fn empty_report(rewritten: RewrittenQuery) -> SearchReport {
    SearchReport {
        hits: Vec::new(),
        rewritten_query: rewritten.text,
        query_rewrites: rewritten.rewrites,
        domains: Vec::new(),
        top_score: 0.0,
        token_coverage: 0.0,
        abstained: true,
    }
}

fn build_domain_index(spec: CorpusSpec) -> Option<DomainIndex> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(spec.csv.as_bytes());
    let headers = reader.headers().ok()?.clone();
    let header_indices: HashMap<&str, usize> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| (header, index))
        .collect();
    let mut documents = Vec::new();

    for (record_index, record) in reader.records().enumerate() {
        let Ok(record) = record else {
            continue;
        };
        if spec.exclude_deprecated_styles
            && value(&record, &header_indices, "Status").eq_ignore_ascii_case("deprecated")
        {
            continue;
        }

        let identity = first_value(&record, &header_indices, spec.identity_fields)
            .unwrap_or_else(|| format!("{}-{}", spec.domain, record_index + 1));
        let search_text = join_fields(&record, &header_indices, spec.search_fields);
        let tokens = tokenize(&search_text);
        if tokens.is_empty() {
            continue;
        }

        let mut term_frequencies = HashMap::new();
        for token in &tokens {
            *term_frequencies.entry(token.clone()).or_insert(0) += 1;
        }
        documents.push(KnowledgeDocument {
            normalized_identity: normalize_identity(&identity),
            identity: identity.clone(),
            term_frequencies,
            length: tokens.len(),
            search_text,
            location: format!(
                "{ASSET_ROOT}/data/{}:{}",
                spec.relative_path,
                record_index + 2
            ),
            excerpt: build_excerpt(&record, &header_indices, &identity, &spec),
        });
    }

    if documents.is_empty() {
        return None;
    }

    let mut document_frequencies = HashMap::new();
    for document in &documents {
        for token in document.term_frequencies.keys() {
            *document_frequencies.entry(token.clone()).or_insert(0) += 1;
        }
    }
    let average_length = documents
        .iter()
        .map(|document| document.length)
        .sum::<usize>() as f64
        / documents.len() as f64;

    Some(DomainIndex {
        domain: spec.domain.to_string(),
        label: spec.label.to_string(),
        documents,
        document_frequencies,
        average_length: average_length.max(1.0),
    })
}

fn bm25_score(index: &DomainIndex, document: &KnowledgeDocument, query_tokens: &[&String]) -> f64 {
    let document_count = index.documents.len() as f64;
    query_tokens
        .iter()
        .map(|token| {
            let frequency = document
                .term_frequencies
                .get(token.as_str())
                .copied()
                .unwrap_or_default() as f64;
            if frequency == 0.0 {
                return 0.0;
            }
            let document_frequency = index
                .document_frequencies
                .get(token.as_str())
                .copied()
                .unwrap_or_default() as f64;
            let idf = ((document_count - document_frequency + 0.5) / (document_frequency + 0.5)
                + 1.0)
                .ln();
            let length_normalization =
                1.0 - BM25_B + BM25_B * document.length as f64 / index.average_length;
            idf * frequency * (BM25_K1 + 1.0) / (frequency + BM25_K1 * length_normalization)
        })
        .sum()
}

fn identity_bonus(raw_query: &str, query_tokens: &[String], document: &KnowledgeDocument) -> f64 {
    if normalize_identity(raw_query) == document.normalized_identity {
        return 5.0;
    }

    let identity_tokens = tokenize(&document.identity);
    if identity_tokens.len() < 2 {
        return 0.0;
    }
    let query_tokens: HashSet<&str> = query_tokens.iter().map(String::as_str).collect();
    let matched = identity_tokens
        .iter()
        .filter(|token| query_tokens.contains(token.as_str()))
        .count();
    if matched as f64 / identity_tokens.len() as f64 >= 0.66 {
        1.5
    } else {
        0.0
    }
}

fn select_diverse_hits(candidates: Vec<Candidate>, limit: usize) -> Vec<KnowledgeHit> {
    let mut selected = Vec::new();
    let mut selected_domains = HashSet::new();
    let mut selected_identities = HashSet::new();

    // 第一轮每个数据域只取一条，防止同一张表占满 top-k。
    for candidate in &candidates {
        if selected.len() >= limit {
            break;
        }
        if selected_domains.contains(candidate.domain.as_str())
            || selected_identities.contains(candidate.identity.as_str())
        {
            continue;
        }
        selected_domains.insert(candidate.domain.clone());
        selected_identities.insert(candidate.identity.clone());
        selected.push(candidate.hit.clone());
    }

    // 数据域不足时允许同域补位，但仍按身份字段去重。
    if selected.len() < limit {
        for candidate in candidates {
            if selected.len() >= limit {
                break;
            }
            if selected_identities.insert(candidate.identity) {
                selected.push(candidate.hit);
            }
        }
    }
    selected
}

#[derive(Debug)]
struct RewrittenQuery {
    text: String,
    tokens: Vec<String>,
    rewrites: Vec<String>,
}

fn rewrite_query(query: &str) -> RewrittenQuery {
    let normalized = query.to_lowercase();
    let mut tokens = tokenize(query);
    let mut additions = Vec::new();
    let mut rewrites = Vec::new();

    for (phrase, expansion) in lexicon::ZH_TO_EN_EXPANSIONS {
        if normalized.contains(phrase) {
            additions.extend(expansion.iter().copied());
            rewrites.push(format!("{}=>{}", phrase, expansion.join(",")));
        }
    }
    for (phrase, expansion) in lexicon::EN_SYNONYMS {
        if contains_token_or_phrase(&normalized, phrase) {
            additions.extend(expansion.iter().copied());
            rewrites.push(format!("{}=>{}", phrase, expansion.join(",")));
        }
    }

    for addition in &additions {
        tokens.extend(tokenize(addition));
    }
    let tokens = ordered_unique(tokens);
    let text = if additions.is_empty() {
        query.trim().to_string()
    } else {
        format!("{} {}", query.trim(), additions.join(" "))
    };

    RewrittenQuery {
        text,
        tokens,
        rewrites,
    }
}

#[derive(Debug)]
struct DomainRouting {
    domains: HashSet<String>,
    strong_domains: HashSet<String>,
}

fn route_domains(query: &str, tokens: &[String], action: UiuxAction) -> DomainRouting {
    let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
    let normalized = query.to_lowercase();
    let mut domains = HashSet::new();
    let mut strong_domains = HashSet::new();

    let action_domains: &[&str] = match action {
        UiuxAction::Beautify => &["style", "product"],
        UiuxAction::Describe => &["style", "product"],
        UiuxAction::Audit => &["ux", "web", "style"],
        UiuxAction::DesignSystem => &["reasoning", "product", "style", "color", "typography", "ux"],
    };
    domains.extend(action_domains.iter().map(|domain| (*domain).to_string()));

    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "color",
        &["color", "palette", "accent", "contrast", "gradient"],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "typography",
        &["font", "typography", "heading", "serif", "sans"],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "chart",
        &["chart", "graph", "visualization", "trend", "dashboard"],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "landing",
        &["landing", "hero", "cta", "conversion", "pricing"],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "icons",
        &["icon", "icons", "lucide", "phosphor", "glyph"],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "motion",
        &[
            "animation",
            "motion",
            "transition",
            "parallax",
            "canvas",
            "compositor",
            "performance",
        ],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "ux",
        &[
            "accessibility",
            "usability",
            "responsive",
            "focus",
            "keyboard",
            "performance",
        ],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "web",
        &["aria", "form", "input", "touch", "mobile", "responsive"],
    );
    add_domain_for_tokens(
        &mut domains,
        &mut strong_domains,
        &token_set,
        "react",
        &["react", "nextjs", "rerender", "bundle", "suspense"],
    );

    for (alias, domain) in stack_aliases() {
        if contains_token_or_phrase(&normalized, alias) {
            domains.insert((*domain).to_string());
            strong_domains.insert((*domain).to_string());
        }
    }

    DomainRouting {
        domains,
        strong_domains,
    }
}

fn add_domain_for_tokens(
    domains: &mut HashSet<String>,
    strong_domains: &mut HashSet<String>,
    tokens: &HashSet<&str>,
    domain: &str,
    hints: &[&str],
) {
    if hints.iter().any(|hint| tokens.contains(hint)) {
        domains.insert(domain.to_string());
        strong_domains.insert(domain.to_string());
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            buffer.push(character.to_ascii_lowercase());
        } else {
            push_token(&mut tokens, &mut buffer);
        }
    }
    push_token(&mut tokens, &mut buffer);
    tokens
}

fn push_token(tokens: &mut Vec<String>, buffer: &mut String) {
    if !buffer.is_empty() && !is_stopword(buffer) {
        tokens.push(std::mem::take(buffer));
    } else {
        buffer.clear();
    }
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "for"
            | "from"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
            | "with"
            | "kiss"
            | "yagni"
            | "solid"
    )
}

fn contains_token_or_phrase(text: &str, phrase: &str) -> bool {
    if phrase.chars().any(|character| character.is_whitespace()) {
        return text.contains(phrase);
    }
    tokenize(text).iter().any(|token| token == phrase)
}

fn first_value(
    record: &csv::StringRecord,
    headers: &HashMap<&str, usize>,
    fields: &[&str],
) -> Option<String> {
    fields
        .iter()
        .map(|field| value(record, headers, field).trim())
        .find(|candidate| !candidate.is_empty())
        .map(str::to_string)
}

fn value<'a>(
    record: &'a csv::StringRecord,
    headers: &HashMap<&str, usize>,
    field: &str,
) -> &'a str {
    headers
        .get(field)
        .and_then(|index| record.get(*index))
        .unwrap_or_default()
}

fn join_fields(
    record: &csv::StringRecord,
    headers: &HashMap<&str, usize>,
    fields: &[&str],
) -> String {
    fields
        .iter()
        .map(|field| value(record, headers, field).trim())
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_excerpt(
    record: &csv::StringRecord,
    headers: &HashMap<&str, usize>,
    identity: &str,
    spec: &CorpusSpec,
) -> String {
    let mut lines = vec![
        format!("知识域：{} ({})", spec.label, spec.domain),
        format!("条目：{}", compact_whitespace(identity)),
    ];
    for field in spec.display_fields {
        let field_value = compact_whitespace(value(record, headers, field));
        if field_value.is_empty() || field_value == identity {
            continue;
        }
        lines.push(format!("{}：{}", localized_field(field), field_value));
    }
    truncate_text(&lines.join("\n"), MAX_EXCERPT_CHARS)
}

fn localized_field(field: &str) -> &str {
    match field {
        "Style Category" => "风格",
        "Product Type" | "UI_Category" => "产品类型",
        "Keywords" | "Mood/Style Keywords" => "关键词",
        "Primary Colors" | "Primary" => "主色",
        "Effects & Animation" | "Key_Effects" => "效果与动效",
        "Best For" => "适用场景",
        "Do Not Use For" => "不适用场景",
        "Performance" | "Performance Notes" => "性能",
        "Accessibility" => "无障碍",
        "Complexity" => "复杂度",
        "AI Prompt Keywords" => "提示词关键词",
        "CSS/Technical Keywords" => "技术关键词",
        "Primary Style Recommendation" | "Style_Priority" => "主风格",
        "Secondary Styles" => "辅助风格",
        "Landing Page Pattern" | "Recommended_Pattern" => "页面模式",
        "Dashboard Style (if applicable)" => "仪表盘风格",
        "Color Palette Focus" | "Color_Mood" => "配色方向",
        "Typography_Mood" => "字体方向",
        "Key Considerations" | "Description" => "要点",
        "Category" => "类别",
        "Issue" | "Guideline" => "规则",
        "Platform" | "Applies To" => "适用范围",
        "Do" => "应当",
        "Don't" => "避免",
        "Severity" => "级别",
        "Intensity Tier" => "动效强度",
        "Trigger" => "触发",
        "Duration" => "时长",
        "Easing" => "缓动",
        "Heading Font" => "标题字体",
        "Body Font" => "正文字体",
        "Font Pairing Name" => "字体组合",
        "Data Type" => "数据类型",
        "Best Chart Type" => "推荐图表",
        "Pattern Name" => "落地页模式",
        "Decision_Rules" => "决策规则",
        "Anti_Patterns" => "反模式",
        "Status" => "状态",
        "Verified At" => "核验日期",
        _ => field,
    }
}

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    format!("{}...", text.chars().take(max_chars).collect::<String>())
}

fn normalize_identity(text: &str) -> String {
    tokenize(text).join(" ")
}

fn ordered_unique<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn corpus_specs() -> Vec<CorpusSpec> {
    let mut specs = vec![
        CorpusSpec {
            domain: "style",
            label: "视觉风格",
            relative_path: "styles.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/styles.csv"),
            identity_fields: &["Style ID", "Style Category", "Aliases"],
            search_fields: STYLE_SEARCH,
            display_fields: STYLE_DISPLAY,
            exclude_deprecated_styles: true,
        },
        CorpusSpec {
            domain: "product",
            label: "产品模板",
            relative_path: "products.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/products.csv"),
            identity_fields: &["Product Type"],
            search_fields: PRODUCT_SEARCH,
            display_fields: PRODUCT_DISPLAY,
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "motion",
            label: "动效与性能",
            relative_path: "motion.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/motion.csv"),
            identity_fields: &["Category", "Intensity Tier"],
            search_fields: MOTION_SEARCH,
            display_fields: MOTION_DISPLAY,
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "ux",
            label: "UX 指南",
            relative_path: "ux-guidelines.csv",
            csv: include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/ux-guidelines.csv"
            ),
            identity_fields: &["Issue", "Category"],
            search_fields: GUIDELINE_SEARCH,
            display_fields: GUIDELINE_DISPLAY,
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "web",
            label: "应用界面指南",
            relative_path: "app-interface.csv",
            csv: include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/app-interface.csv"
            ),
            identity_fields: &["Issue", "Category"],
            search_fields: GUIDELINE_SEARCH,
            display_fields: GUIDELINE_DISPLAY,
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "react",
            label: "React 性能指南",
            relative_path: "react-performance.csv",
            csv: include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/react-performance.csv"
            ),
            identity_fields: &["Issue", "Category"],
            search_fields: GUIDELINE_SEARCH,
            display_fields: GUIDELINE_DISPLAY,
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "color",
            label: "色彩方案",
            relative_path: "colors.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/colors.csv"),
            identity_fields: &["Product Type"],
            search_fields: &["Product Type", "Notes"],
            display_fields: &[
                "Product Type",
                "Primary",
                "Secondary",
                "Accent",
                "Background",
                "Foreground",
                "Border",
                "Notes",
            ],
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "typography",
            label: "字体组合",
            relative_path: "typography.csv",
            csv: include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/typography.csv"
            ),
            identity_fields: &["Font Pairing Name"],
            search_fields: &[
                "Font Pairing Name",
                "Category",
                "Mood/Style Keywords",
                "Best For",
                "Heading Font",
                "Body Font",
            ],
            display_fields: &[
                "Font Pairing Name",
                "Category",
                "Heading Font",
                "Body Font",
                "Mood/Style Keywords",
                "Best For",
                "Notes",
            ],
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "landing",
            label: "落地页模式",
            relative_path: "landing.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/landing.csv"),
            identity_fields: &["Pattern ID", "Pattern Name", "Aliases"],
            search_fields: &[
                "Pattern ID",
                "Pattern Name",
                "Aliases",
                "Keywords",
                "Conversion Optimization",
                "Section Order",
            ],
            display_fields: &[
                "Pattern Name",
                "Keywords",
                "Section Order",
                "Primary CTA Placement",
                "Color Strategy",
                "Recommended Effects",
                "Conversion Optimization",
            ],
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "chart",
            label: "图表选择",
            relative_path: "charts.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/charts.csv"),
            identity_fields: &["Data Type", "Best Chart Type"],
            search_fields: &[
                "Data Type",
                "Keywords",
                "Best Chart Type",
                "When to Use",
                "When NOT to Use",
                "Accessibility Notes",
            ],
            display_fields: &[
                "Data Type",
                "Keywords",
                "Best Chart Type",
                "Secondary Options",
                "When to Use",
                "When NOT to Use",
                "Color Guidance",
                "Accessibility Notes",
            ],
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "icons",
            label: "图标语义",
            relative_path: "icons.csv",
            csv: include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/icons.csv"),
            identity_fields: &["Icon Name", "Category"],
            search_fields: &["Category", "Icon Name", "Keywords", "Best For", "Library"],
            display_fields: &[
                "Category",
                "Icon Name",
                "Keywords",
                "Library",
                "Usage",
                "Best For",
                "Semantic Role",
                "Allowed Contexts",
            ],
            exclude_deprecated_styles: false,
        },
        CorpusSpec {
            domain: "reasoning",
            label: "设计推理",
            relative_path: "ui-reasoning.csv",
            csv: include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/ui-reasoning.csv"
            ),
            identity_fields: &["UI_Category"],
            search_fields: &[
                "UI_Category",
                "Recommended_Pattern",
                "Style_Priority",
                "Color_Mood",
                "Typography_Mood",
                "Key_Effects",
                "Decision_Rules",
                "Anti_Patterns",
            ],
            display_fields: &[
                "UI_Category",
                "Recommended_Pattern",
                "Style_Priority",
                "Color_Mood",
                "Typography_Mood",
                "Key_Effects",
                "Decision_Rules",
                "Anti_Patterns",
                "Confidence",
            ],
            exclude_deprecated_styles: false,
        },
    ];
    specs.extend(stack_specs());
    specs
}

macro_rules! stack_spec {
    ($domain:literal, $label:literal, $file:literal, $csv:expr) => {
        CorpusSpec {
            domain: $domain,
            label: $label,
            relative_path: concat!("stacks/", $file),
            csv: $csv,
            identity_fields: &["Guideline", "Category"],
            search_fields: STACK_SEARCH,
            display_fields: STACK_DISPLAY,
            exclude_deprecated_styles: false,
        }
    };
}

fn stack_specs() -> Vec<CorpusSpec> {
    vec![
        stack_spec!(
            "stack:angular",
            "Angular",
            "angular.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/angular.csv")
        ),
        stack_spec!(
            "stack:astro",
            "Astro",
            "astro.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/astro.csv")
        ),
        stack_spec!(
            "stack:avalonia",
            "Avalonia",
            "avalonia.csv",
            include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/avalonia.csv"
            )
        ),
        stack_spec!(
            "stack:flutter",
            "Flutter",
            "flutter.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/flutter.csv")
        ),
        stack_spec!(
            "stack:html-tailwind",
            "HTML + Tailwind",
            "html-tailwind.csv",
            include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/html-tailwind.csv"
            )
        ),
        stack_spec!(
            "stack:javafx",
            "JavaFX",
            "javafx.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/javafx.csv")
        ),
        stack_spec!(
            "stack:jetpack-compose",
            "Jetpack Compose",
            "jetpack-compose.csv",
            include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/jetpack-compose.csv"
            )
        ),
        stack_spec!(
            "stack:laravel",
            "Laravel",
            "laravel.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/laravel.csv")
        ),
        stack_spec!(
            "stack:nextjs",
            "Next.js",
            "nextjs.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/nextjs.csv")
        ),
        stack_spec!(
            "stack:nuxt-ui",
            "Nuxt UI",
            "nuxt-ui.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/nuxt-ui.csv")
        ),
        stack_spec!(
            "stack:nuxtjs",
            "Nuxt",
            "nuxtjs.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/nuxtjs.csv")
        ),
        stack_spec!(
            "stack:react-native",
            "React Native",
            "react-native.csv",
            include_str!(
                "../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/react-native.csv"
            )
        ),
        stack_spec!(
            "stack:react",
            "React",
            "react.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/react.csv")
        ),
        stack_spec!(
            "stack:shadcn",
            "shadcn/ui",
            "shadcn.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/shadcn.csv")
        ),
        stack_spec!(
            "stack:svelte",
            "Svelte",
            "svelte.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/svelte.csv")
        ),
        stack_spec!(
            "stack:swiftui",
            "SwiftUI",
            "swiftui.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/swiftui.csv")
        ),
        stack_spec!(
            "stack:threejs",
            "Three.js",
            "threejs.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/threejs.csv")
        ),
        stack_spec!(
            "stack:uno",
            "Uno Platform",
            "uno.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/uno.csv")
        ),
        stack_spec!(
            "stack:uwp",
            "UWP",
            "uwp.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/uwp.csv")
        ),
        stack_spec!(
            "stack:vue",
            "Vue",
            "vue.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/vue.csv")
        ),
        stack_spec!(
            "stack:winui",
            "WinUI",
            "winui.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/winui.csv")
        ),
        stack_spec!(
            "stack:wpf",
            "WPF",
            "wpf.csv",
            include_str!("../../../assets/resources/ui-ux-pro-max-v2.15.0/data/stacks/wpf.csv")
        ),
    ]
}

fn stack_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("angular", "stack:angular"),
        ("astro", "stack:astro"),
        ("avalonia", "stack:avalonia"),
        ("flutter", "stack:flutter"),
        ("tailwind", "stack:html-tailwind"),
        ("javafx", "stack:javafx"),
        ("jetpack compose", "stack:jetpack-compose"),
        ("laravel", "stack:laravel"),
        ("next.js", "stack:nextjs"),
        ("nextjs", "stack:nextjs"),
        ("nuxt ui", "stack:nuxt-ui"),
        ("nuxt", "stack:nuxtjs"),
        ("react native", "stack:react-native"),
        ("react", "stack:react"),
        ("shadcn", "stack:shadcn"),
        ("svelte", "stack:svelte"),
        ("swiftui", "stack:swiftui"),
        ("three.js", "stack:threejs"),
        ("threejs", "stack:threejs"),
        ("uno platform", "stack:uno"),
        ("uwp", "stack:uwp"),
        ("vue", "stack:vue"),
        ("winui", "stack:winui"),
        ("wpf", "stack:wpf"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_QUERY: &str = "查找鼠标动效主题美化：黑洞吞噬小星球、地球月球卫星战舰公转系统、深邃暗黑多层星空、巨手破空抓握地球拖入裂缝，保持 KISS/YAGNI 极简几何科技感与高性能 Direct2D/Canvas 实现";

    #[test]
    fn v215_catalog_is_loaded_as_structured_documents() {
        assert!(document_count() > 1_500, "应加载核心域与 22 个技术栈数据");
        assert!(materialized_markdown().contains("UI/UX Pro Max v2.15.0"));
        assert!(materialized_markdown().contains("HUD / Sci-Fi FUI"));
    }

    #[test]
    fn narrative_space_query_returns_diverse_relevant_hits() {
        let report = search(USER_QUERY, UiuxAction::Beautify, 3);
        let excerpts = report
            .hits
            .iter()
            .map(|hit| hit.excerpt.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(report.hits.len(), 3, "原始长查询应稳定返回 3 条知识");
        assert!(
            report
                .hits
                .iter()
                .any(|hit| hit.domain == "style" && hit.excerpt.contains("HUD / Sci-Fi FUI")),
            "top-3 的 style 域应命中 HUD / Sci-Fi FUI，实际为: {excerpts}"
        );
        assert!(
            report.hits.iter().any(
                |hit| hit.domain == "product" && hit.excerpt.contains("Space Tech / Aerospace")
            ),
            "top-3 的 product 域应命中 Space Tech / Aerospace，实际为: {excerpts}"
        );
        assert!(
            report.hits.iter().any(|hit| hit.domain == "motion"),
            "top-3 应包含 motion 性能/动效知识，实际为: {:?}",
            report.domains
        );
        assert!(!report.abstained);
    }

    #[test]
    fn exact_style_identity_has_priority() {
        let report = search("HUD / Sci-Fi FUI", UiuxAction::Beautify, 1);
        assert_eq!(report.hits.len(), 1);
        assert!(report.hits[0].excerpt.contains("HUD / Sci-Fi FUI"));
    }

    #[test]
    fn explicit_stack_query_can_retrieve_svelte_guidance() {
        let report = search(
            "Svelte animation performance state update",
            UiuxAction::Audit,
            5,
        );
        assert!(
            report.hits.iter().any(|hit| hit.domain == "stack:svelte"),
            "显式 Svelte 查询应召回对应技术栈规则: {:?}",
            report.domains
        );
    }

    #[test]
    fn unrelated_query_abstains_instead_of_returning_noise() {
        let report = search("ZXQJ-9471 lunar tax ledger", UiuxAction::Beautify, 3);
        assert!(report.hits.is_empty());
        assert!(report.abstained);
    }

    #[test]
    fn locations_are_versioned_and_traceable() {
        let report = search("glassmorphism", UiuxAction::Beautify, 1);
        assert!(report.hits[0]
            .location
            .contains("ui-ux-pro-max-v2.15.0/data/styles.csv:"));
    }
}
