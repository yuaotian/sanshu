// UI/UX Pro Max Rust 原生实现
// 负责数据加载、搜索引擎、设计系统生成与输出格式化

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use chrono::Local;
use once_cell::sync::Lazy;
use regex::Regex;
use rust_embed::RustEmbed;
use serde::Serialize;
use serde_json::Value;

use crate::mcp::tools::memory::TextSimilarity;
use crate::log_debug;
use super::lexicon;
use super::sanitize::{sanitize_path_segment, sanitize_slug};

const MAX_RESULTS: usize = 3;
const BOX_WIDTH: usize = 90;

#[derive(RustEmbed)]
#[folder = "skills/ui-ux-pro-max/data"]
struct EmbeddedUiuxData;

#[derive(Clone)]
struct DomainConfig {
    file: &'static str,
    search_cols: &'static [&'static str],
    output_cols: &'static [&'static str],
}

static DOMAIN_CONFIGS: Lazy<HashMap<&'static str, DomainConfig>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "style",
        DomainConfig {
            file: "styles.csv",
            search_cols: &["Style Category", "Keywords", "Best For", "Type"],
            output_cols: &[
                "Style Category",
                "Type",
                "Keywords",
                "Primary Colors",
                "Effects & Animation",
                "Best For",
                "Performance",
                "Accessibility",
                "Framework Compatibility",
                "Complexity",
            ],
        },
    );
    map.insert(
        "prompt",
        DomainConfig {
            file: "prompts.csv",
            search_cols: &[
                "Style Category",
                "AI Prompt Keywords (Copy-Paste Ready)",
                "CSS/Technical Keywords",
            ],
            output_cols: &[
                "Style Category",
                "AI Prompt Keywords (Copy-Paste Ready)",
                "CSS/Technical Keywords",
                "Implementation Checklist",
            ],
        },
    );
    map.insert(
        "color",
        DomainConfig {
            file: "colors.csv",
            search_cols: &["Product Type", "Keywords", "Notes"],
            output_cols: &[
                "Product Type",
                "Keywords",
                "Primary (Hex)",
                "Secondary (Hex)",
                "CTA (Hex)",
                "Background (Hex)",
                "Text (Hex)",
                "Border (Hex)",
                "Notes",
            ],
        },
    );
    map.insert(
        "chart",
        DomainConfig {
            file: "charts.csv",
            search_cols: &["Data Type", "Keywords", "Best Chart Type", "Accessibility Notes"],
            output_cols: &[
                "Data Type",
                "Keywords",
                "Best Chart Type",
                "Secondary Options",
                "Color Guidance",
                "Accessibility Notes",
                "Library Recommendation",
                "Interactive Level",
            ],
        },
    );
    map.insert(
        "landing",
        DomainConfig {
            file: "landing.csv",
            search_cols: &["Pattern Name", "Keywords", "Conversion Optimization", "Section Order"],
            output_cols: &[
                "Pattern Name",
                "Keywords",
                "Section Order",
                "Primary CTA Placement",
                "Color Strategy",
                "Conversion Optimization",
            ],
        },
    );
    map.insert(
        "product",
        DomainConfig {
            file: "products.csv",
            search_cols: &[
                "Product Type",
                "Keywords",
                "Primary Style Recommendation",
                "Key Considerations",
            ],
            output_cols: &[
                "Product Type",
                "Keywords",
                "Primary Style Recommendation",
                "Secondary Styles",
                "Landing Page Pattern",
                "Dashboard Style (if applicable)",
                "Color Palette Focus",
            ],
        },
    );
    map.insert(
        "ux",
        DomainConfig {
            file: "ux-guidelines.csv",
            search_cols: &["Category", "Issue", "Description", "Platform"],
            output_cols: &[
                "Category",
                "Issue",
                "Platform",
                "Description",
                "Do",
                "Don't",
                "Code Example Good",
                "Code Example Bad",
                "Severity",
            ],
        },
    );
    map.insert(
        "typography",
        DomainConfig {
            file: "typography.csv",
            search_cols: &[
                "Font Pairing Name",
                "Category",
                "Mood/Style Keywords",
                "Best For",
                "Heading Font",
                "Body Font",
            ],
            output_cols: &[
                "Font Pairing Name",
                "Category",
                "Heading Font",
                "Body Font",
                "Mood/Style Keywords",
                "Best For",
                "Google Fonts URL",
                "CSS Import",
                "Tailwind Config",
                "Notes",
            ],
        },
    );
    map.insert(
        "icons",
        DomainConfig {
            file: "icons.csv",
            search_cols: &["Category", "Icon Name", "Keywords", "Best For"],
            output_cols: &[
                "Category",
                "Icon Name",
                "Keywords",
                "Library",
                "Import Code",
                "Usage",
                "Best For",
                "Style",
            ],
        },
    );
    map.insert(
        "react",
        DomainConfig {
            file: "react-performance.csv",
            search_cols: &["Category", "Issue", "Keywords", "Description"],
            output_cols: &[
                "Category",
                "Issue",
                "Platform",
                "Description",
                "Do",
                "Don't",
                "Code Example Good",
                "Code Example Bad",
                "Severity",
            ],
        },
    );
    map.insert(
        "web",
        DomainConfig {
            file: "web-interface.csv",
            search_cols: &["Category", "Issue", "Keywords", "Description"],
            output_cols: &[
                "Category",
                "Issue",
                "Platform",
                "Description",
                "Do",
                "Don't",
                "Code Example Good",
                "Code Example Bad",
                "Severity",
            ],
        },
    );
    map
});

static STACK_CONFIGS: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert("html-tailwind", "stacks/html-tailwind.csv");
    map.insert("react", "stacks/react.csv");
    map.insert("nextjs", "stacks/nextjs.csv");
    map.insert("vue", "stacks/vue.csv");
    map.insert("nuxtjs", "stacks/nuxtjs.csv");
    map.insert("nuxt-ui", "stacks/nuxt-ui.csv");
    map.insert("svelte", "stacks/svelte.csv");
    map.insert("swiftui", "stacks/swiftui.csv");
    map.insert("react-native", "stacks/react-native.csv");
    map.insert("flutter", "stacks/flutter.csv");
    map.insert("shadcn", "stacks/shadcn.csv");
    map.insert("jetpack-compose", "stacks/jetpack-compose.csv");
    map
});

static STACK_SEARCH_COLS: &[&str] = &["Category", "Guideline", "Description", "Do", "Don't"];
static STACK_OUTPUT_COLS: &[&str] = &[
    "Category",
    "Guideline",
    "Description",
    "Do",
    "Don't",
    "Code Good",
    "Code Bad",
    "Severity",
    "Docs URL",
];

static DOMAIN_KEYWORDS: Lazy<Vec<(&'static str, &'static [&'static str])>> = Lazy::new(|| {
    vec![
        ("color", &["color", "palette", "hex", "#", "rgb"]),
        (
            "chart",
            &[
                "chart", "graph", "visualization", "trend", "bar", "pie", "scatter", "heatmap",
                "funnel",
            ],
        ),
        (
            "landing",
            &[
                "landing", "page", "cta", "conversion", "hero", "testimonial", "pricing",
                "section",
            ],
        ),
        (
            "product",
            &[
                "saas", "ecommerce", "e-commerce", "fintech", "healthcare", "gaming",
                "portfolio", "crypto", "dashboard",
            ],
        ),
        (
            "prompt",
            &["prompt", "css", "implementation", "variable", "checklist", "tailwind"],
        ),
        (
            "style",
            &[
                "style", "design", "ui", "minimalism", "glassmorphism", "neumorphism",
                "brutalism", "dark mode", "flat", "aurora",
            ],
        ),
        (
            "ux",
            &[
                "ux", "usability", "accessibility", "wcag", "touch", "scroll", "animation",
                "keyboard", "navigation", "mobile",
            ],
        ),
        (
            "typography",
            &["font", "typography", "heading", "serif", "sans"],
        ),
        (
            "icons",
            &["icon", "icons", "lucide", "heroicons", "symbol", "glyph", "pictogram", "svg icon"],
        ),
        (
            "react",
            &[
                "react", "next.js", "nextjs", "suspense", "memo", "usecallback", "useeffect",
                "rerender", "bundle", "waterfall", "barrel", "dynamic import", "rsc",
                "server component",
            ],
        ),
        (
            "web",
            &["aria", "focus", "outline", "semantic", "virtualize", "autocomplete", "form", "input type", "preconnect"],
        ),
    ]
});

static TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\w\s]").unwrap());

static KEYWORD_STOPWORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "and", "or", "the", "with", "for", "from", "into", "that", "this", "these", "those",
        "best", "use", "using", "used", "good", "bad", "new", "all", "any", "your", "their",
    ]
    .into_iter()
    .collect()
});

#[derive(Clone)]
struct BM25 {
    k1: f64,
    b: f64,
    corpus: Vec<Vec<String>>,
    doc_lengths: Vec<usize>,
    avgdl: f64,
    idf: HashMap<String, f64>,
    doc_freqs: HashMap<String, usize>,
}

impl BM25 {
    fn new(k1: f64, b: f64) -> Self {
        Self {
            k1,
            b,
            corpus: Vec::new(),
            doc_lengths: Vec::new(),
            avgdl: 0.0,
            idf: HashMap::new(),
            doc_freqs: HashMap::new(),
        }
    }

    fn tokenize(text: &str) -> Vec<String> {
        let lower = text.to_lowercase();
        let cleaned = TOKEN_RE.replace_all(&lower, " ");
        cleaned
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .map(|w| w.to_string())
            .collect()
    }

    fn fit(&mut self, documents: &[String]) {
        self.corpus = documents.iter().map(|doc| Self::tokenize(doc)).collect();
        self.doc_lengths = self.corpus.iter().map(|doc| doc.len()).collect();
        self.idf.clear();
        self.doc_freqs.clear();

        let n = self.corpus.len();
        if n == 0 {
            self.avgdl = 0.0;
            return;
        }

        self.avgdl = self.doc_lengths.iter().sum::<usize>() as f64 / n as f64;

        for doc in &self.corpus {
            let mut seen: HashSet<&str> = HashSet::new();
            for token in doc {
                if seen.insert(token) {
                    *self.doc_freqs.entry(token.clone()).or_insert(0) += 1;
                }
            }
        }

        for (token, freq) in &self.doc_freqs {
            let freq = *freq as f64;
            let n = n as f64;
            let idf = ((n - freq + 0.5) / (freq + 0.5) + 1.0).ln();
            self.idf.insert(token.clone(), idf);
        }
    }

    fn score(&self, query: &str) -> Vec<(usize, f64)> {
        let query_tokens = Self::tokenize(query);
        let mut scores = Vec::with_capacity(self.corpus.len());

        for (idx, doc) in self.corpus.iter().enumerate() {
            let mut term_freqs: HashMap<&str, usize> = HashMap::new();
            for token in doc {
                *term_freqs.entry(token).or_insert(0) += 1;
            }

            let doc_len = self.doc_lengths.get(idx).copied().unwrap_or(0) as f64;
            let mut score = 0.0;

            for token in &query_tokens {
                if let Some(idf) = self.idf.get(token) {
                    let tf = *term_freqs.get(token.as_str()).unwrap_or(&0) as f64;
                    if tf == 0.0 || self.avgdl == 0.0 {
                        continue;
                    }
                    let numerator = tf * (self.k1 + 1.0);
                    let denominator = tf + self.k1 * (1.0 - self.b + self.b * doc_len / self.avgdl);
                    score += idf * numerator / denominator;
                }
            }

            scores.push((idx, score));
        }

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
    }
}

// ============ Query Expansion & Suggest Tokenization ============

/// 查询扩展最多追加的 token 数，避免过度扩展导致噪声/性能问题。
const QUERY_EXPANSION_MAX_TOKENS: usize = 32;

/// BM25 无结果时启用相似度回退：若 Top1 相似度低于该值，认为没有可信结果（避免胡乱返回）。
const FUZZY_FALLBACK_MIN_TOP1: f64 = 0.35;

/// 相似度回退时的最小入选分数（低于该值的候选直接忽略）。
const FUZZY_FALLBACK_MIN_ITEM: f64 = 0.25;

fn is_allowed_short_token(token: &str) -> bool {
    matches!(token, "ui" | "ux" | "ai")
}

/// 提取 ASCII 连续单词，用于处理中英粘连（如 `UI美化` -> `ui`）。
fn extract_ascii_words(text_lower: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut curr = String::new();

    for ch in text_lower.chars() {
        if ch.is_ascii_alphanumeric() {
            curr.push(ch.to_ascii_lowercase());
        } else if !curr.is_empty() {
            out.push(std::mem::take(&mut curr));
        }
    }
    if !curr.is_empty() {
        out.push(curr);
    }

    out
}

/// `uiux_suggest` 专用分词：
/// - 复用 `TOKEN_RE` 清理标点
/// - 保留长度 > 2 token，同时允许 `ui/ux/ai` 这类 2 字母高频信号
/// - 额外提取 ASCII 连续单词，兼容中英粘连输入（如 `UI美化`）
fn tokenize_for_suggest(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let cleaned = TOKEN_RE.replace_all(&lower, " ");

    let mut out: Vec<String> = cleaned
        .split_whitespace()
        .filter(|w| w.len() > 2 || is_allowed_short_token(w))
        .map(|w| w.to_string())
        .collect();

    // 补充：处理中英粘连（如 `ui美化`）
    for w in extract_ascii_words(&lower) {
        if w.len() > 2 || is_allowed_short_token(&w) {
            out.push(w);
        }
    }

    out
}

/// 英文轻量词干化（非常保守），用于 query expansion。
///
/// 说明：只对 ASCII token 生效；避免对非英文 token 做不安全的字符串切片。
fn stem_en_token(token: &str) -> Option<String> {
    if token.len() <= 4 || !token.is_ascii() {
        return None;
    }

    // 常见后缀（优先处理更长的后缀）
    for suffix in ["ness", "ment", "tion", "sion", "able", "ible", "ingly", "edly", "ing", "ed", "ly", "ity"] {
        if token.ends_with(suffix) && token.len() > suffix.len() + 2 {
            return Some(token[..token.len() - suffix.len()].to_string());
        }
    }

    // 复数（跳过 glass/class 这类以 ss 结尾的词）
    if token.ends_with("es") && token.len() > 5 {
        return Some(token[..token.len() - 2].to_string());
    }
    if token.ends_with('s') && token.len() > 5 && !token.ends_with("ss") {
        return Some(token[..token.len() - 1].to_string());
    }

    None
}

/// 收集 Query Expansion 追加 token（去重、排序）。
fn collect_query_expansion(query: &str) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    let query_lower = query.to_lowercase();

    // 1) ASCII 信号（支持中英粘连，如 `UI美化`）
    for w in extract_ascii_words(&query_lower) {
        match w.as_str() {
            // `ui/ux` 在 BM25 tokenizer 中会被过滤（len==2），因此映射到更长的概念词
            "ui" | "uiux" => {
                set.insert("design".to_string());
                set.insert("interface".to_string());
            }
            "ux" => {
                set.insert("usability".to_string());
                set.insert("accessibility".to_string());
            }
            _ => {}
        }
    }

    // 2) 中文短语 -> 英文 token
    for (zh, ens) in lexicon::ZH_TO_EN_EXPANSIONS {
        if query.contains(zh) {
            for &en in *ens {
                set.insert(en.to_string());
            }
        }
    }

    // 3) 英文 token -> 同义/相关 token + 轻量词干
    // 说明：BM25::tokenize 已做 lower + 清理标点，可直接用于等值匹配。
    for token in BM25::tokenize(&query_lower) {
        // 同义词扩展
        for (key, syns) in lexicon::EN_SYNONYMS {
            if token == *key {
                for &syn in *syns {
                    set.insert(syn.to_string());
                }
            }
        }
        // 词干化扩展（保守）
        if let Some(stem) = stem_en_token(&token) {
            set.insert(stem);
        }
    }

    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    if out.len() > QUERY_EXPANSION_MAX_TOKENS {
        out.truncate(QUERY_EXPANSION_MAX_TOKENS);
    }
    out
}

/// 构造 BM25 用的扩展查询串：原 query + 追加 token。
fn expand_query_for_bm25(query: &str) -> String {
    let extra = collect_query_expansion(query);
    if extra.is_empty() {
        return query.to_string();
    }
    format!("{} {}", query, extra.join(" "))
}

struct DomainIndex {
    file: &'static str,
    output_cols: &'static [&'static str],
    rows: Vec<HashMap<String, String>>,
    bm25: BM25,
    /// 原始文档文本（由 `search_cols` 拼接），用于 BM25 fit 与相似度回退。
    documents: Vec<String>,
}

impl DomainIndex {
    fn new(config: &DomainConfig) -> Result<Self, String> {
        let rows = load_csv(config.file)?;
        let documents: Vec<String> = rows
            .iter()
            .map(|row| {
                config
                    .search_cols
                    .iter()
                    .map(|col| row.get(*col).cloned().unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();

        let mut bm25 = BM25::new(1.5, 0.75);
        bm25.fit(&documents);

        Ok(Self {
            file: config.file,
            output_cols: config.output_cols,
            rows,
            bm25,
            documents,
        })
    }

    fn search(&self, query: &str, max_results: usize) -> Vec<HashMap<String, String>> {
        let max_results = max_results.max(1);

        // 1) Query Expansion：先把中文/同义概念映射成额外英文 token，再喂给 BM25
        let expanded_query = expand_query_for_bm25(query);
        let ranked = self.bm25.score(&expanded_query);
        let mut results = Vec::new();

        for (idx, score) in ranked {
            if results.len() >= max_results {
                break;
            }
            if score <= 0.0 {
                continue;
            }
            if let Some(row) = self.rows.get(idx) {
                let mut out = HashMap::new();
                for col in self.output_cols {
                    if let Some(value) = row.get(*col) {
                        out.insert((*col).to_string(), value.clone());
                    }
                }
                results.push(out);
            }
        }

        // 2) 语义回退：BM25 没命中时，使用轻量文本相似度做兜底（数据规模百级，性能可控）
        if !results.is_empty() {
            return results;
        }

        let mut scored: Vec<(f64, usize)> = Vec::with_capacity(self.documents.len());
        for (idx, doc) in self.documents.iter().enumerate() {
            if doc.trim().is_empty() {
                continue;
            }
            let sim = TextSimilarity::calculate_enhanced(&expanded_query, doc);
            scored.push((sim, idx));
        }

        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let best = scored.first().map(|(s, _)| *s).unwrap_or(0.0);
        if best < FUZZY_FALLBACK_MIN_TOP1 {
            return Vec::new();
        }

        for (sim, idx) in scored {
            if results.len() >= max_results {
                break;
            }
            if sim < FUZZY_FALLBACK_MIN_ITEM {
                break; // 已按降序排序
            }
            if let Some(row) = self.rows.get(idx) {
                let mut out = HashMap::new();
                for col in self.output_cols {
                    if let Some(value) = row.get(*col) {
                        out.insert((*col).to_string(), value.clone());
                    }
                }
                results.push(out);
            }
        }

        results
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub domain: String,
    pub query: String,
    pub file: Option<String>,
    pub count: usize,
    pub results: Vec<HashMap<String, String>>,
    pub stack: Option<String>,
    pub error: Option<String>,
}

impl SearchResult {
    fn error(domain: &str, query: &str, message: &str) -> Self {
        Self {
            domain: domain.to_string(),
            query: query.to_string(),
            file: None,
            count: 0,
            results: Vec::new(),
            stack: None,
            error: Some(message.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SuggestResult {
    pub should_suggest: bool,
    pub score: usize,
    pub matched_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BeautifyResult {
    pub style: Vec<HashMap<String, String>>,
    pub color: Vec<HashMap<String, String>>,
    pub typography: Vec<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistSummary {
    pub design_system_dir: String,
    pub created_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DesignSystemOutput {
    pub design_system: DesignSystem,
    pub persisted: Option<PersistSummary>,
    pub formatted: String,
}

struct UiuxStore {
    domains: HashMap<&'static str, DomainIndex>,
    stacks: HashMap<&'static str, DomainIndex>,
    reasoning: Vec<HashMap<String, String>>,
    keyword_set: HashSet<String>,
}

static UIUX_STORE: Lazy<UiuxStore> = Lazy::new(UiuxStore::load);

impl UiuxStore {
    fn load() -> Self {
        let mut domains = HashMap::new();
        for (name, config) in DOMAIN_CONFIGS.iter() {
            match DomainIndex::new(config) {
                Ok(index) => {
                    domains.insert(*name, index);
                }
                Err(err) => {
                    log_debug!("UIUX 域数据加载失败: {} -> {}", name, err);
                }
            }
        }

        let mut stacks = HashMap::new();
        for (name, file) in STACK_CONFIGS.iter() {
            let config = DomainConfig {
                file,
                search_cols: STACK_SEARCH_COLS,
                output_cols: STACK_OUTPUT_COLS,
            };
            match DomainIndex::new(&config) {
                Ok(index) => {
                    stacks.insert(*name, index);
                }
                Err(err) => {
                    log_debug!("UIUX 栈数据加载失败: {} -> {}", name, err);
                }
            }
        }

        let reasoning = load_csv("ui-reasoning.csv").unwrap_or_default();
        let keyword_set = build_keyword_set();

        Self {
            domains,
            stacks,
            reasoning,
            keyword_set,
        }
    }
}

fn read_embedded(path: &str) -> Result<Vec<u8>, String> {
    EmbeddedUiuxData::get(path)
        .map(|file| file.data.into_owned())
        .ok_or_else(|| format!("未找到内嵌资源: {}", path))
}

fn load_csv(path: &str) -> Result<Vec<HashMap<String, String>>, String> {
    let bytes = read_embedded(path)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(Cursor::new(bytes));

    let headers = reader
        .headers()
        .map_err(|e| format!("读取 CSV 头失败: {}", e))?
        .clone();

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| format!("读取 CSV 记录失败: {}", e))?;
        let mut row = HashMap::new();
        for (idx, value) in record.iter().enumerate() {
            if let Some(key) = headers.get(idx) {
                row.insert(key.to_string(), value.to_string());
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

fn detect_domain(query: &str) -> &'static str {
    let query_lower = query.to_lowercase();

    // 中文/中英混合：先用轻量映射做域提示（命中则直接返回）
    // 说明：这里不做复杂 NLP，只做可维护的短语命中。
    let mut zh_scores: HashMap<&'static str, usize> = HashMap::new();
    for (hint, domain) in lexicon::ZH_DOMAIN_HINTS {
        let hit = if hint.is_ascii() {
            query_lower.contains(&hint.to_lowercase())
        } else {
            query.contains(hint)
        };
        if hit {
            *zh_scores.entry(*domain).or_insert(0) += 1;
        }
    }
    if let Some((best, score)) = zh_scores.into_iter().max_by_key(|(_, s)| *s) {
        if score > 0 {
            return best;
        }
    }

    let mut best_domain = "style";
    let mut best_score = 0;

    for (domain, keywords) in DOMAIN_KEYWORDS.iter() {
        let score = keywords
            .iter()
            .filter(|kw| query_lower.contains(&kw.to_lowercase()))
            .count();
        if score > best_score {
            best_score = score;
            best_domain = domain;
        }
    }

    if best_score == 0 {
        "style"
    } else {
        best_domain
    }
}

fn normalize_format(value: Option<&str>, default: &str) -> String {
    value
        .unwrap_or(default)
        .trim()
        .to_lowercase()
}

/// 结果数量上限控制，避免过大输出
pub fn cap_max_results(value: Option<u32>, cap: u32, default: u32) -> usize {
    let cap = cap.max(1);
    let raw = value.unwrap_or(default).max(1);
    raw.min(cap) as usize
}

pub fn search_domain(query: &str, domain: Option<&str>, max_results: Option<usize>) -> SearchResult {
    let store = &*UIUX_STORE;
    let requested_domain = domain.unwrap_or_else(|| detect_domain(query));
    let domain = if store.domains.contains_key(requested_domain) {
        requested_domain
    } else {
        "style"
    };
    let max_results = max_results.unwrap_or(MAX_RESULTS);

    match store.domains.get(domain) {
        Some(index) => {
            let results = index.search(query, max_results);
            SearchResult {
                domain: domain.to_string(),
                query: query.to_string(),
                file: Some(index.file.to_string()),
                count: results.len(),
                results,
                stack: None,
                error: None,
            }
        }
        None => SearchResult::error(domain, query, &format!("未知领域: {}", domain)),
    }
}

pub fn search_stack(query: &str, stack: &str, max_results: Option<usize>) -> SearchResult {
    let store = &*UIUX_STORE;
    let max_results = max_results.unwrap_or(MAX_RESULTS);

    match store.stacks.get(stack) {
        Some(index) => {
            let results = index.search(query, max_results);
            SearchResult {
                domain: "stack".to_string(),
                query: query.to_string(),
                file: Some(index.file.to_string()),
                count: results.len(),
                results,
                stack: Some(stack.to_string()),
                error: None,
            }
        }
        None => SearchResult::error(
            "stack",
            query,
            &format!("Unknown stack: {}. Available: {}", stack, available_stacks().join(", ")),
        ),
    }
}

fn available_stacks() -> Vec<&'static str> {
    let mut stacks: Vec<&'static str> = STACK_CONFIGS.keys().copied().collect();
    stacks.sort();
    stacks
}

pub fn format_search_output(result: &SearchResult) -> String {
    if let Some(err) = &result.error {
        return format!("Error: {}", err);
    }

    let mut output = Vec::new();
    if let Some(stack) = &result.stack {
        output.push("## UI Pro Max Stack Guidelines".to_string());
        output.push(format!("**Stack:** {} | **Query:** {}", stack, result.query));
    } else {
        output.push("## UI Pro Max Search Results".to_string());
        output.push(format!("**Domain:** {} | **Query:** {}", result.domain, result.query));
    }
    output.push(format!(
        "**Source:** {} | **Found:** {} results\n",
        result.file.clone().unwrap_or_default(),
        result.count
    ));

    for (idx, row) in result.results.iter().enumerate() {
        output.push(format!("### Result {}", idx + 1));
        // 使用输出列顺序保证稳定展示
        if let Some(config) = DOMAIN_CONFIGS.get(result.domain.as_str()) {
            for col in config.output_cols {
                if let Some(value) = row.get(*col) {
                    let mut value_str = value.clone();
                    if value_str.len() > 300 {
                        value_str.truncate(300);
                        value_str.push_str("...");
                    }
                    output.push(format!("- **{}:** {}", col, value_str));
                }
            }
        } else if result.domain == "stack" {
            for col in STACK_OUTPUT_COLS {
                if let Some(value) = row.get(*col) {
                    let mut value_str = value.clone();
                    if value_str.len() > 300 {
                        value_str.truncate(300);
                        value_str.push_str("...");
                    }
                    output.push(format!("- **{}:** {}", col, value_str));
                }
            }
        } else {
            for (key, value) in row {
                let mut value_str = value.clone();
                if value_str.len() > 300 {
                    value_str.truncate(300);
                    value_str.push_str("...");
                }
                output.push(format!("- **{}:** {}", key, value_str));
            }
        }
        output.push(String::new());
    }

    output.join("\n")
}

pub fn format_search_json(result: &SearchResult) -> Result<String, String> {
    serde_json::to_string_pretty(result).map_err(|e| format!("JSON 序列化失败: {}", e))
}

/// UI 提示词美化：基于现有数据组合，避免重复
pub fn beautify_prompt(query: &str, max_results: usize) -> BeautifyResult {
    let limit = max_results.max(1);
    let style = search_domain(query, Some("style"), Some(limit));
    let color = search_domain(query, Some("color"), Some(limit));
    let typography = search_domain(query, Some("typography"), Some(limit));

    BeautifyResult {
        style: dedupe_results(style.results),
        color: dedupe_results(color.results),
        typography: dedupe_results(typography.results),
    }
}

fn dedupe_results(rows: Vec<HashMap<String, String>>) -> Vec<HashMap<String, String>> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for row in rows {
        let mut items: Vec<_> = row.iter().collect();
        items.sort_by(|a, b| a.0.cmp(b.0));
        let signature = items
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("|");
        if seen.insert(signature) {
            output.push(row);
        }
    }
    output
}

#[derive(Debug, Clone, Serialize)]
pub struct PatternInfo {
    pub name: String,
    pub sections: String,
    pub cta_placement: String,
    pub color_strategy: String,
    pub conversion: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StyleInfo {
    pub name: String,
    pub style_type: String,
    pub effects: String,
    pub keywords: String,
    pub best_for: String,
    pub performance: String,
    pub accessibility: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColorInfo {
    pub primary: String,
    pub secondary: String,
    pub cta: String,
    pub background: String,
    pub text: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypographyInfo {
    pub heading: String,
    pub body: String,
    pub mood: String,
    pub best_for: String,
    pub google_fonts_url: String,
    pub css_import: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignSystem {
    pub project_name: String,
    pub category: String,
    pub pattern: PatternInfo,
    pub style: StyleInfo,
    pub colors: ColorInfo,
    pub typography: TypographyInfo,
    pub key_effects: String,
    pub anti_patterns: String,
    pub decision_rules: Value,
    pub severity: String,
}

#[derive(Debug, Clone)]
struct ReasoningResult {
    pattern: String,
    style_priority: Vec<String>,
    color_mood: String,
    typography_mood: String,
    key_effects: String,
    anti_patterns: String,
    decision_rules: Value,
    severity: String,
}

struct DesignSystemGenerator {
    reasoning_data: Vec<HashMap<String, String>>,
}

impl DesignSystemGenerator {
    fn new() -> Self {
        let store = &*UIUX_STORE;
        Self {
            reasoning_data: store.reasoning.clone(),
        }
    }

    fn find_reasoning_rule(&self, category: &str) -> Option<&HashMap<String, String>> {
        let category_lower = category.to_lowercase();
        for rule in &self.reasoning_data {
            if rule
                .get("UI_Category")
                .map(|v| v.to_lowercase())
                == Some(category_lower.clone())
            {
                return Some(rule);
            }
        }

        for rule in &self.reasoning_data {
            let ui_cat = rule
                .get("UI_Category")
                .map(|v| v.to_lowercase())
                .unwrap_or_default();
            if ui_cat.contains(&category_lower) || category_lower.contains(&ui_cat) {
                return Some(rule);
            }
        }

        for rule in &self.reasoning_data {
            let ui_cat = rule
                .get("UI_Category")
                .map(|v| v.to_lowercase())
                .unwrap_or_default();
            let keywords = ui_cat.replace(['/', '-'], " ");
            for kw in keywords.split_whitespace() {
                if category_lower.contains(kw) {
                    return Some(rule);
                }
            }
        }

        None
    }

    fn apply_reasoning(&self, category: &str) -> ReasoningResult {
        let rule = self.find_reasoning_rule(category);
        if rule.is_none() {
            return ReasoningResult {
                pattern: "Hero + Features + CTA".to_string(),
                style_priority: vec!["Minimalism".to_string(), "Flat Design".to_string()],
                color_mood: "Professional".to_string(),
                typography_mood: "Clean".to_string(),
                key_effects: "Subtle hover transitions".to_string(),
                anti_patterns: String::new(),
                decision_rules: Value::Object(serde_json::Map::new()),
                severity: "MEDIUM".to_string(),
            };
        }

        let rule = rule.unwrap();
        let decision_rules = rule
            .get("Decision_Rules")
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        let style_priority = rule
            .get("Style_Priority")
            .unwrap_or(&String::new())
            .split('+')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        ReasoningResult {
            pattern: rule.get("Recommended_Pattern").cloned().unwrap_or_default(),
            style_priority,
            color_mood: rule.get("Color_Mood").cloned().unwrap_or_default(),
            typography_mood: rule.get("Typography_Mood").cloned().unwrap_or_default(),
            key_effects: rule.get("Key_Effects").cloned().unwrap_or_default(),
            anti_patterns: rule.get("Anti_Patterns").cloned().unwrap_or_default(),
            decision_rules,
            severity: rule
                .get("Severity")
                .cloned()
                .unwrap_or_else(|| "MEDIUM".to_string()),
        }
    }

    fn select_best_match(
        &self,
        results: &[HashMap<String, String>],
        priority_keywords: &[String],
    ) -> Option<HashMap<String, String>> {
        if results.is_empty() {
            return None;
        }
        if priority_keywords.is_empty() {
            return Some(results[0].clone());
        }

        for priority in priority_keywords {
            let priority_lower = priority.to_lowercase();
            for result in results {
                let style_name = result
                    .get("Style Category")
                    .map(|v| v.to_lowercase())
                    .unwrap_or_default();
                if style_name.contains(&priority_lower) || priority_lower.contains(&style_name) {
                    return Some(result.clone());
                }
            }
        }

        let mut scored: Vec<(i32, &HashMap<String, String>)> = Vec::new();
        for result in results {
            let result_str = format!("{:?}", result).to_lowercase();
            let mut score = 0;
            for kw in priority_keywords {
                let kw_lower = kw.to_lowercase();
                if let Some(style_name) = result.get("Style Category") {
                    if style_name.to_lowercase().contains(&kw_lower) {
                        score += 10;
                        continue;
                    }
                }
                if let Some(keywords) = result.get("Keywords") {
                    if keywords.to_lowercase().contains(&kw_lower) {
                        score += 3;
                        continue;
                    }
                }
                if result_str.contains(&kw_lower) {
                    score += 1;
                }
            }
            scored.push((score, result));
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        if let Some((score, best)) = scored.first() {
            if *score > 0 {
                return Some((*best).clone());
            }
        }

        Some(results[0].clone())
    }

    fn multi_domain_search(&self, query: &str, style_priority: &[String]) -> HashMap<String, SearchResult> {
        let mut results = HashMap::new();
        for (domain, config) in [
            ("product", 1usize),
            ("style", 3usize),
            ("color", 2usize),
            ("landing", 2usize),
            ("typography", 2usize),
        ] {
            if domain == "style" && !style_priority.is_empty() {
                let priority_query = style_priority.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
                let combined_query = format!("{} {}", query, priority_query);
                results.insert(
                    domain.to_string(),
                    search_domain(&combined_query, Some(domain), Some(config)),
                );
            } else {
                results.insert(
                    domain.to_string(),
                    search_domain(query, Some(domain), Some(config)),
                );
            }
        }
        results
    }

    fn generate(&self, query: &str, project_name: Option<&str>) -> DesignSystem {
        let product_result = search_domain(query, Some("product"), Some(1));
        let mut category = "General".to_string();
        if let Some(first) = product_result.results.first() {
            if let Some(value) = first.get("Product Type") {
                if !value.is_empty() {
                    category = value.clone();
                }
            }
        }

        let reasoning = self.apply_reasoning(&category);
        let style_priority = reasoning.style_priority.clone();
        let mut search_results = self.multi_domain_search(query, &style_priority);
        search_results.insert("product".to_string(), product_result);

        let style_results = search_results
            .get("style")
            .map(|r| r.results.clone())
            .unwrap_or_default();
        let color_results = search_results
            .get("color")
            .map(|r| r.results.clone())
            .unwrap_or_default();
        let typography_results = search_results
            .get("typography")
            .map(|r| r.results.clone())
            .unwrap_or_default();
        let landing_results = search_results
            .get("landing")
            .map(|r| r.results.clone())
            .unwrap_or_default();

        let best_style = self
            .select_best_match(&style_results, &style_priority)
            .unwrap_or_default();
        let best_color = color_results.first().cloned().unwrap_or_default();
        let best_typography = typography_results.first().cloned().unwrap_or_default();
        let best_landing = landing_results.first().cloned().unwrap_or_default();

        let style_effects = best_style
            .get("Effects & Animation")
            .cloned()
            .unwrap_or_default();
        let reasoning_effects = reasoning.key_effects.clone();
        let combined_effects = if !style_effects.is_empty() {
            style_effects.clone()
        } else {
            reasoning_effects
        };

        DesignSystem {
            project_name: project_name.unwrap_or(query).to_uppercase(),
            category: category.clone(),
            pattern: PatternInfo {
                name: best_landing
                    .get("Pattern Name")
                    .cloned()
                    .unwrap_or_else(|| reasoning.pattern.clone()),
                sections: best_landing
                    .get("Section Order")
                    .cloned()
                    .unwrap_or_else(|| "Hero > Features > CTA".to_string()),
                cta_placement: best_landing
                    .get("Primary CTA Placement")
                    .cloned()
                    .unwrap_or_else(|| "Above fold".to_string()),
                color_strategy: best_landing
                    .get("Color Strategy")
                    .cloned()
                    .unwrap_or_default(),
                conversion: best_landing
                    .get("Conversion Optimization")
                    .cloned()
                    .unwrap_or_default(),
            },
            style: StyleInfo {
                name: best_style
                    .get("Style Category")
                    .cloned()
                    .unwrap_or_else(|| "Minimalism".to_string()),
                style_type: best_style
                    .get("Type")
                    .cloned()
                    .unwrap_or_else(|| "General".to_string()),
                effects: style_effects,
                keywords: best_style.get("Keywords").cloned().unwrap_or_default(),
                best_for: best_style.get("Best For").cloned().unwrap_or_default(),
                performance: best_style.get("Performance").cloned().unwrap_or_default(),
                accessibility: best_style.get("Accessibility").cloned().unwrap_or_default(),
            },
            colors: ColorInfo {
                primary: best_color
                    .get("Primary (Hex)")
                    .cloned()
                    .unwrap_or_else(|| "#2563EB".to_string()),
                secondary: best_color
                    .get("Secondary (Hex)")
                    .cloned()
                    .unwrap_or_else(|| "#3B82F6".to_string()),
                cta: best_color
                    .get("CTA (Hex)")
                    .cloned()
                    .unwrap_or_else(|| "#F97316".to_string()),
                background: best_color
                    .get("Background (Hex)")
                    .cloned()
                    .unwrap_or_else(|| "#F8FAFC".to_string()),
                text: best_color
                    .get("Text (Hex)")
                    .cloned()
                    .unwrap_or_else(|| "#1E293B".to_string()),
                notes: best_color.get("Notes").cloned().unwrap_or_default(),
            },
            typography: TypographyInfo {
                heading: best_typography
                    .get("Heading Font")
                    .cloned()
                    .unwrap_or_else(|| "Inter".to_string()),
                body: best_typography
                    .get("Body Font")
                    .cloned()
                    .unwrap_or_else(|| "Inter".to_string()),
                mood: best_typography
                    .get("Mood/Style Keywords")
                    .cloned()
                    .unwrap_or_else(|| reasoning.typography_mood.clone()),
                best_for: best_typography
                    .get("Best For")
                    .cloned()
                    .unwrap_or_default(),
                google_fonts_url: best_typography
                    .get("Google Fonts URL")
                    .cloned()
                    .unwrap_or_default(),
                css_import: best_typography.get("CSS Import").cloned().unwrap_or_default(),
            },
            key_effects: combined_effects,
            anti_patterns: reasoning.anti_patterns.clone(),
            decision_rules: reasoning.decision_rules.clone(),
            severity: reasoning.severity.clone(),
        }
    }
}

fn wrap_text(text: &str, prefix: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = prefix.to_string();
    for word in text.split_whitespace() {
        if current.len() + word.len() + 1 <= width - 2 {
            if current != prefix {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            if current != prefix {
                lines.push(current.clone());
            }
            current = format!("{}{}", prefix, word);
        }
    }
    if current != prefix {
        lines.push(current);
    }
    lines
}

pub fn format_ascii_box(design_system: &DesignSystem) -> String {
    let mut lines = Vec::new();
    let w = BOX_WIDTH - 1;

    lines.push(format!("+{}+", "-".repeat(w)));
    lines.push(format!(
        "|  TARGET: {} - RECOMMENDED DESIGN SYSTEM",
        design_system.project_name
    ));
    lines.push(format!("+{}+", "-".repeat(w)));
    lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));

    // Pattern
    lines.push(format!("|  PATTERN: {}", design_system.pattern.name));
    if !design_system.pattern.conversion.is_empty() {
        lines.push(format!("|     Conversion: {}", design_system.pattern.conversion));
    }
    if !design_system.pattern.cta_placement.is_empty() {
        lines.push(format!("|     CTA: {}", design_system.pattern.cta_placement));
    }
    lines.push("|     Sections:".to_string());
    for (idx, section) in design_system.pattern.sections.split('>').map(|s| s.trim()).filter(|s| !s.is_empty()).enumerate() {
        lines.push(format!("|       {}. {}", idx + 1, section));
    }
    lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));

    // Style
    lines.push(format!("|  STYLE: {}", design_system.style.name));
    if !design_system.style.keywords.is_empty() {
        for line in wrap_text(&format!("Keywords: {}", design_system.style.keywords), "|     ", BOX_WIDTH) {
            lines.push(line);
        }
    }
    if !design_system.style.best_for.is_empty() {
        for line in wrap_text(&format!("Best For: {}", design_system.style.best_for), "|     ", BOX_WIDTH) {
            lines.push(line);
        }
    }
    if !design_system.style.performance.is_empty() || !design_system.style.accessibility.is_empty() {
        let perf_a11y = format!(
            "Performance: {} | Accessibility: {}",
            design_system.style.performance,
            design_system.style.accessibility
        );
        lines.push(format!("|     {}", perf_a11y));
    }
    lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));

    // Colors
    lines.push("|  COLORS:".to_string());
    lines.push(format!("|     Primary:    {}", design_system.colors.primary));
    lines.push(format!("|     Secondary:  {}", design_system.colors.secondary));
    lines.push(format!("|     CTA:        {}", design_system.colors.cta));
    lines.push(format!("|     Background: {}", design_system.colors.background));
    lines.push(format!("|     Text:       {}", design_system.colors.text));
    if !design_system.colors.notes.is_empty() {
        for line in wrap_text(&format!("Notes: {}", design_system.colors.notes), "|     ", BOX_WIDTH) {
            lines.push(line);
        }
    }
    lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));

    // Typography
    lines.push(format!(
        "|  TYPOGRAPHY: {} / {}",
        design_system.typography.heading,
        design_system.typography.body
    ));
    if !design_system.typography.mood.is_empty() {
        for line in wrap_text(&format!("Mood: {}", design_system.typography.mood), "|     ", BOX_WIDTH) {
            lines.push(line);
        }
    }
    if !design_system.typography.best_for.is_empty() {
        for line in wrap_text(&format!("Best For: {}", design_system.typography.best_for), "|     ", BOX_WIDTH) {
            lines.push(line);
        }
    }
    if !design_system.typography.google_fonts_url.is_empty() {
        lines.push(format!("|     Google Fonts: {}", design_system.typography.google_fonts_url));
    }
    if !design_system.typography.css_import.is_empty() {
        let mut css = design_system.typography.css_import.clone();
        if css.len() > 70 {
            css.truncate(70);
            css.push_str("...");
        }
        lines.push(format!("|     CSS Import: {}", css));
    }
    lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));

    // Key Effects
    if !design_system.key_effects.is_empty() {
        lines.push("|  KEY EFFECTS:".to_string());
        for line in wrap_text(&design_system.key_effects, "|     ", BOX_WIDTH) {
            lines.push(line);
        }
        lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));
    }

    // Anti-patterns
    if !design_system.anti_patterns.is_empty() {
        lines.push("|  AVOID (Anti-patterns):".to_string());
        for line in wrap_text(&design_system.anti_patterns, "|     ", BOX_WIDTH) {
            lines.push(line);
        }
        lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));
    }

    // Checklist
    lines.push("|  PRE-DELIVERY CHECKLIST:".to_string());
    let checklist_items = [
        "[ ] No emojis as icons (use SVG: Heroicons/Lucide)",
        "[ ] cursor-pointer on all clickable elements",
        "[ ] Hover states with smooth transitions (150-300ms)",
        "[ ] Light mode: text contrast 4.5:1 minimum",
        "[ ] Focus states visible for keyboard nav",
        "[ ] prefers-reduced-motion respected",
        "[ ] Responsive: 375px, 768px, 1024px, 1440px",
    ];
    for item in checklist_items {
        lines.push(format!("|     {}", item));
    }
    lines.push(format!("|{}", " ".repeat(BOX_WIDTH)));

    lines.push(format!("+{}+", "-".repeat(w)));

    // 对齐宽度
    lines
        .into_iter()
        .map(|line| {
            if line.starts_with('+') {
                line
            } else {
                format!("{:<width$}|", line, width = BOX_WIDTH)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_markdown(design_system: &DesignSystem) -> String {
    let mut lines = Vec::new();
    lines.push(format!("## Design System: {}", design_system.project_name));
    lines.push(String::new());

    lines.push("### Pattern".to_string());
    lines.push(format!("- **Name:** {}", design_system.pattern.name));
    if !design_system.pattern.conversion.is_empty() {
        lines.push(format!("- **Conversion Focus:** {}", design_system.pattern.conversion));
    }
    if !design_system.pattern.cta_placement.is_empty() {
        lines.push(format!("- **CTA Placement:** {}", design_system.pattern.cta_placement));
    }
    if !design_system.pattern.color_strategy.is_empty() {
        lines.push(format!("- **Color Strategy:** {}", design_system.pattern.color_strategy));
    }
    lines.push(format!("- **Sections:** {}", design_system.pattern.sections));
    lines.push(String::new());

    lines.push("### Style".to_string());
    lines.push(format!("- **Name:** {}", design_system.style.name));
    if !design_system.style.keywords.is_empty() {
        lines.push(format!("- **Keywords:** {}", design_system.style.keywords));
    }
    if !design_system.style.best_for.is_empty() {
        lines.push(format!("- **Best For:** {}", design_system.style.best_for));
    }
    if !design_system.style.performance.is_empty() || !design_system.style.accessibility.is_empty() {
        lines.push(format!(
            "- **Performance:** {} | **Accessibility:** {}",
            design_system.style.performance,
            design_system.style.accessibility
        ));
    }
    lines.push(String::new());

    lines.push("### Colors".to_string());
    lines.push("| Role | Hex |".to_string());
    lines.push("|------|-----|".to_string());
    lines.push(format!("| Primary | {} |", design_system.colors.primary));
    lines.push(format!("| Secondary | {} |", design_system.colors.secondary));
    lines.push(format!("| CTA | {} |", design_system.colors.cta));
    lines.push(format!("| Background | {} |", design_system.colors.background));
    lines.push(format!("| Text | {} |", design_system.colors.text));
    if !design_system.colors.notes.is_empty() {
        lines.push(String::new());
        lines.push(format!("*Notes: {}*", design_system.colors.notes));
    }
    lines.push(String::new());

    lines.push("### Typography".to_string());
    lines.push(format!("- **Heading:** {}", design_system.typography.heading));
    lines.push(format!("- **Body:** {}", design_system.typography.body));
    if !design_system.typography.mood.is_empty() {
        lines.push(format!("- **Mood:** {}", design_system.typography.mood));
    }
    if !design_system.typography.best_for.is_empty() {
        lines.push(format!("- **Best For:** {}", design_system.typography.best_for));
    }
    if !design_system.typography.google_fonts_url.is_empty() {
        lines.push(format!("- **Google Fonts:** {}", design_system.typography.google_fonts_url));
    }
    if !design_system.typography.css_import.is_empty() {
        lines.push("- **CSS Import:**".to_string());
        lines.push("```css".to_string());
        lines.push(design_system.typography.css_import.clone());
        lines.push("```".to_string());
    }
    lines.push(String::new());

    if !design_system.key_effects.is_empty() {
        lines.push("### Key Effects".to_string());
        lines.push(design_system.key_effects.clone());
        lines.push(String::new());
    }

    if !design_system.anti_patterns.is_empty() {
        lines.push("### Avoid (Anti-patterns)".to_string());
        let replaced = design_system
            .anti_patterns
            .replace(" + ", "\n- ");
        lines.push(format!("- {}", replaced));
        lines.push(String::new());
    }

    lines.push("### Pre-Delivery Checklist".to_string());
    lines.push("- [ ] No emojis as icons (use SVG instead)".to_string());
    lines.push("- [ ] All icons from consistent icon set (Heroicons/Lucide)".to_string());
    lines.push("- [ ] `cursor-pointer` on all clickable elements".to_string());
    lines.push("- [ ] Hover states with smooth transitions (150-300ms)".to_string());
    lines.push("- [ ] Light mode: text contrast 4.5:1 minimum".to_string());
    lines.push("- [ ] Focus states visible for keyboard navigation".to_string());
    lines.push("- [ ] `prefers-reduced-motion` respected".to_string());
    lines.push("- [ ] Responsive: 375px, 768px, 1024px, 1440px".to_string());
    lines.push(String::new());

    lines.join("\n")
}

fn format_master_md(design_system: &DesignSystem) -> String {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut lines = Vec::new();

    lines.push("# Design System Master File".to_string());
    lines.push(String::new());
    lines.push("> **LOGIC:** When building a specific page, first check `design-system/pages/[page-name].md`.".to_string());
    lines.push("> If that file exists, its rules **override** this Master file.".to_string());
    lines.push("> If not, strictly follow the rules below.".to_string());
    lines.push(String::new());
    lines.push("---".to_string());
    lines.push(String::new());
    lines.push(format!("**Project:** {}", design_system.project_name));
    lines.push(format!("**Generated:** {}", timestamp));
    lines.push(format!("**Category:** {}", design_system.category));
    lines.push(String::new());
    lines.push("---".to_string());
    lines.push(String::new());

    lines.push("## Global Rules".to_string());
    lines.push(String::new());

    lines.push("### Color Palette".to_string());
    lines.push(String::new());
    lines.push("| Role | Hex | CSS Variable |".to_string());
    lines.push("|------|-----|--------------|".to_string());
    lines.push(format!("| Primary | `{}` | `--color-primary` |", design_system.colors.primary));
    lines.push(format!("| Secondary | `{}` | `--color-secondary` |", design_system.colors.secondary));
    lines.push(format!("| CTA/Accent | `{}` | `--color-cta` |", design_system.colors.cta));
    lines.push(format!("| Background | `{}` | `--color-background` |", design_system.colors.background));
    lines.push(format!("| Text | `{}` | `--color-text` |", design_system.colors.text));
    lines.push(String::new());
    if !design_system.colors.notes.is_empty() {
        lines.push(format!("**Color Notes:** {}", design_system.colors.notes));
        lines.push(String::new());
    }

    lines.push("### Typography".to_string());
    lines.push(String::new());
    lines.push(format!("- **Heading Font:** {}", design_system.typography.heading));
    lines.push(format!("- **Body Font:** {}", design_system.typography.body));
    if !design_system.typography.mood.is_empty() {
        lines.push(format!("- **Mood:** {}", design_system.typography.mood));
    }
    if !design_system.typography.google_fonts_url.is_empty() {
        lines.push(format!(
            "- **Google Fonts:** [{} + {}]({})",
            design_system.typography.heading,
            design_system.typography.body,
            design_system.typography.google_fonts_url
        ));
    }
    lines.push(String::new());
    if !design_system.typography.css_import.is_empty() {
        lines.push("**CSS Import:**".to_string());
        lines.push("```css".to_string());
        lines.push(design_system.typography.css_import.clone());
        lines.push("```".to_string());
        lines.push(String::new());
    }

    lines.push("### Spacing Variables".to_string());
    lines.push(String::new());
    lines.push("| Token | Value | Usage |".to_string());
    lines.push("|-------|-------|-------|".to_string());
    lines.push("| `--space-xs` | `4px` / `0.25rem` | Tight gaps |".to_string());
    lines.push("| `--space-sm` | `8px` / `0.5rem` | Icon gaps, inline spacing |".to_string());
    lines.push("| `--space-md` | `16px` / `1rem` | Standard padding |".to_string());
    lines.push("| `--space-lg` | `24px` / `1.5rem` | Section padding |".to_string());
    lines.push("| `--space-xl` | `32px` / `2rem` | Large gaps |".to_string());
    lines.push("| `--space-2xl` | `48px` / `3rem` | Section margins |".to_string());
    lines.push("| `--space-3xl` | `64px` / `4rem` | Hero padding |".to_string());
    lines.push(String::new());

    lines.push("### Shadow Depths".to_string());
    lines.push(String::new());
    lines.push("| Level | Value | Usage |".to_string());
    lines.push("|-------|-------|-------|".to_string());
    lines.push("| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.05)` | Subtle lift |".to_string());
    lines.push("| `--shadow-md` | `0 4px 6px rgba(0,0,0,0.1)` | Cards, buttons |".to_string());
    lines.push("| `--shadow-lg` | `0 10px 15px rgba(0,0,0,0.1)` | Modals, dropdowns |".to_string());
    lines.push("| `--shadow-xl` | `0 20px 25px rgba(0,0,0,0.15)` | Hero images, featured cards |".to_string());
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("## Component Specs".to_string());
    lines.push(String::new());

    lines.push("### Buttons".to_string());
    lines.push(String::new());
    lines.push("```css".to_string());
    lines.push("/* Primary Button */".to_string());
    lines.push(".btn-primary {".to_string());
    lines.push(format!("  background: {};", design_system.colors.cta));
    lines.push("  color: white;".to_string());
    lines.push("  padding: 12px 24px;".to_string());
    lines.push("  border-radius: 8px;".to_string());
    lines.push("  font-weight: 600;".to_string());
    lines.push("  transition: all 200ms ease;".to_string());
    lines.push("  cursor: pointer;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push(".btn-primary:hover {".to_string());
    lines.push("  opacity: 0.9;".to_string());
    lines.push("  transform: translateY(-1px);".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("/* Secondary Button */".to_string());
    lines.push(".btn-secondary {".to_string());
    lines.push("  background: transparent;".to_string());
    lines.push(format!("  color: {};", design_system.colors.primary));
    lines.push(format!("  border: 2px solid {};", design_system.colors.primary));
    lines.push("  padding: 12px 24px;".to_string());
    lines.push("  border-radius: 8px;".to_string());
    lines.push("  font-weight: 600;".to_string());
    lines.push("  transition: all 200ms ease;".to_string());
    lines.push("  cursor: pointer;".to_string());
    lines.push("}".to_string());
    lines.push("```".to_string());
    lines.push(String::new());

    lines.push("### Cards".to_string());
    lines.push(String::new());
    lines.push("```css".to_string());
    lines.push(".card {".to_string());
    lines.push(format!("  background: {};", design_system.colors.background));
    lines.push("  border-radius: 12px;".to_string());
    lines.push("  padding: 24px;".to_string());
    lines.push("  box-shadow: var(--shadow-md);".to_string());
    lines.push("  transition: all 200ms ease;".to_string());
    lines.push("  cursor: pointer;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push(".card:hover {".to_string());
    lines.push("  box-shadow: var(--shadow-lg);".to_string());
    lines.push("  transform: translateY(-2px);".to_string());
    lines.push("}".to_string());
    lines.push("```".to_string());
    lines.push(String::new());

    lines.push("### Inputs".to_string());
    lines.push(String::new());
    lines.push("```css".to_string());
    lines.push(".input {".to_string());
    lines.push("  padding: 12px 16px;".to_string());
    lines.push("  border: 1px solid #E2E8F0;".to_string());
    lines.push("  border-radius: 8px;".to_string());
    lines.push("  font-size: 16px;".to_string());
    lines.push("  transition: border-color 200ms ease;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push(".input:focus {".to_string());
    lines.push(format!("  border-color: {};", design_system.colors.primary));
    lines.push("  outline: none;".to_string());
    lines.push(format!("  box-shadow: 0 0 0 3px {}20;", design_system.colors.primary));
    lines.push("}".to_string());
    lines.push("```".to_string());
    lines.push(String::new());

    lines.push("### Modals".to_string());
    lines.push(String::new());
    lines.push("```css".to_string());
    lines.push(".modal-overlay {".to_string());
    lines.push("  background: rgba(0, 0, 0, 0.5);".to_string());
    lines.push("  backdrop-filter: blur(4px);".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push(".modal {".to_string());
    lines.push("  background: white;".to_string());
    lines.push("  border-radius: 16px;".to_string());
    lines.push("  padding: 32px;".to_string());
    lines.push("  box-shadow: var(--shadow-xl);".to_string());
    lines.push("  max-width: 500px;".to_string());
    lines.push("  width: 90%;".to_string());
    lines.push("}".to_string());
    lines.push("```".to_string());
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("## Style Guidelines".to_string());
    lines.push(String::new());
    lines.push(format!("**Style:** {}", design_system.style.name));
    lines.push(String::new());
    if !design_system.style.keywords.is_empty() {
        lines.push(format!("**Keywords:** {}", design_system.style.keywords));
        lines.push(String::new());
    }
    if !design_system.style.best_for.is_empty() {
        lines.push(format!("**Best For:** {}", design_system.style.best_for));
        lines.push(String::new());
    }
    if !design_system.key_effects.is_empty() {
        lines.push(format!("**Key Effects:** {}", design_system.key_effects));
        lines.push(String::new());
    }

    lines.push("### Page Pattern".to_string());
    lines.push(String::new());
    lines.push(format!("**Pattern Name:** {}", design_system.pattern.name));
    lines.push(String::new());
    if !design_system.pattern.conversion.is_empty() {
        lines.push(format!("- **Conversion Strategy:** {}", design_system.pattern.conversion));
    }
    if !design_system.pattern.cta_placement.is_empty() {
        lines.push(format!("- **CTA Placement:** {}", design_system.pattern.cta_placement));
    }
    lines.push(format!("- **Section Order:** {}", design_system.pattern.sections));
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("## Anti-Patterns (Do NOT Use)".to_string());
    lines.push(String::new());
    if !design_system.anti_patterns.is_empty() {
        for anti in design_system.anti_patterns.split('+').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            lines.push(format!("- ❌ {}", anti));
        }
    }
    lines.push(String::new());
    lines.push("### Additional Forbidden Patterns".to_string());
    lines.push(String::new());
    lines.push("- ❌ **Emojis as icons** — Use SVG icons (Heroicons, Lucide, Simple Icons)".to_string());
    lines.push("- ❌ **Missing cursor:pointer** — All clickable elements must have cursor:pointer".to_string());
    lines.push("- ❌ **Layout-shifting hovers** — Avoid scale transforms that shift layout".to_string());
    lines.push("- ❌ **Low contrast text** — Maintain 4.5:1 minimum contrast ratio".to_string());
    lines.push("- ❌ **Instant state changes** — Always use transitions (150-300ms)".to_string());
    lines.push("- ❌ **Invisible focus states** — Focus states must be visible for a11y".to_string());
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("## Pre-Delivery Checklist".to_string());
    lines.push(String::new());
    lines.push("Before delivering any UI code, verify:".to_string());
    lines.push(String::new());
    lines.push("- [ ] No emojis used as icons (use SVG instead)".to_string());
    lines.push("- [ ] All icons from consistent icon set (Heroicons/Lucide)".to_string());
    lines.push("- [ ] `cursor-pointer` on all clickable elements".to_string());
    lines.push("- [ ] Hover states with smooth transitions (150-300ms)".to_string());
    lines.push("- [ ] Light mode: text contrast 4.5:1 minimum".to_string());
    lines.push("- [ ] Focus states visible for keyboard navigation".to_string());
    lines.push("- [ ] `prefers-reduced-motion` respected".to_string());
    lines.push("- [ ] Responsive: 375px, 768px, 1024px, 1440px".to_string());
    lines.push("- [ ] No content hidden behind fixed navbars".to_string());
    lines.push("- [ ] No horizontal scroll on mobile".to_string());
    lines.push(String::new());

    lines.join("\n")
}

fn detect_page_type(context: &str, style_results: &[HashMap<String, String>]) -> String {
    let context_lower = context.to_lowercase();
    // 使用切片避免不同关键词数量导致的数组长度类型不一致
    let patterns: &[(&[&str], &str)] = &[
        (
            &[
                "dashboard", "admin", "analytics", "data", "metrics", "stats", "monitor",
                "overview",
            ],
            "Dashboard / Data View",
        ),
        (
            &[
                "checkout", "payment", "cart", "purchase", "order", "billing",
            ],
            "Checkout / Payment",
        ),
        (
            &["settings", "profile", "account", "preferences", "config"],
            "Settings / Profile",
        ),
        (
            &["landing", "marketing", "homepage", "hero", "home", "promo"],
            "Landing / Marketing",
        ),
        (
            &["login", "signin", "signup", "register", "auth", "password"],
            "Authentication",
        ),
        (
            &["pricing", "plans", "subscription", "tiers", "packages"],
            "Pricing / Plans",
        ),
        (
            &["blog", "article", "post", "news", "content", "story"],
            "Blog / Article",
        ),
        (
            &["product", "item", "detail", "pdp", "shop", "store"],
            "Product Detail",
        ),
        (
            &["search", "results", "browse", "filter", "catalog", "list"],
            "Search Results",
        ),
        (
            &["empty", "404", "error", "not found", "zero"],
            "Empty State",
        ),
    ];

    for (keywords, page_type) in patterns {
        if keywords.iter().any(|kw| context_lower.contains(kw)) {
            return page_type.to_string();
        }
    }

    if let Some(style) = style_results.first() {
        let best_for = style
            .get("Best For")
            .map(|v| v.to_lowercase())
            .unwrap_or_default();
        if best_for.contains("dashboard") || best_for.contains("data") {
            return "Dashboard / Data View".to_string();
        }
        if best_for.contains("landing") || best_for.contains("marketing") {
            return "Landing / Marketing".to_string();
        }
    }

    "General".to_string()
}

fn generate_intelligent_overrides(
    page_name: &str,
    page_query: Option<&str>,
) -> HashMap<String, Value> {
    let page_lower = page_name.to_lowercase();
    let query_lower = page_query.unwrap_or("").to_lowercase();
    let combined_context = format!("{} {}", page_lower, query_lower);

    let style_search = search_domain(&combined_context, Some("style"), Some(1));
    let ux_search = search_domain(&combined_context, Some("ux"), Some(3));
    let landing_search = search_domain(&combined_context, Some("landing"), Some(1));

    let style_results = style_search.results;
    let ux_results = ux_search.results;
    let landing_results = landing_search.results;

    let page_type = detect_page_type(&combined_context, &style_results);

    let mut layout = HashMap::new();
    let mut spacing = HashMap::new();
    // 显式类型，避免空 HashMap 无法推断类型
    let typography: HashMap<String, String> = HashMap::new();
    let mut colors = HashMap::new();
    let mut components: Vec<String> = Vec::new();
    let unique_components: Vec<String> = Vec::new();
    let mut recommendations: Vec<String> = Vec::new();

    if let Some(style) = style_results.first() {
        let keywords = style.get("Keywords").cloned().unwrap_or_default();
        let effects = style.get("Effects & Animation").cloned().unwrap_or_default();

        if keywords
            .to_lowercase()
            .split_whitespace()
            .any(|kw| ["data", "dense", "dashboard", "grid"].contains(&kw))
        {
            layout.insert("Max Width".to_string(), "1400px or full-width".to_string());
            layout.insert("Grid".to_string(), "12-column grid for data flexibility".to_string());
            spacing.insert("Content Density".to_string(), "High — optimize for information display".to_string());
        } else if keywords
            .to_lowercase()
            .split_whitespace()
            .any(|kw| ["minimal", "simple", "clean", "single"].contains(&kw))
        {
            layout.insert("Max Width".to_string(), "800px (narrow, focused)".to_string());
            layout.insert("Layout".to_string(), "Single column, centered".to_string());
            spacing.insert("Content Density".to_string(), "Low — focus on clarity".to_string());
        } else {
            layout.insert("Max Width".to_string(), "1200px (standard)".to_string());
            layout.insert("Layout".to_string(), "Full-width sections, centered content".to_string());
        }

        if !effects.is_empty() {
            recommendations.push(format!("Effects: {}", effects));
        }
    }

    for ux in &ux_results {
        let category = ux.get("Category").cloned().unwrap_or_default();
        let do_text = ux.get("Do").cloned().unwrap_or_default();
        let dont_text = ux.get("Don't").cloned().unwrap_or_default();
        if !do_text.is_empty() {
            recommendations.push(format!("{}: {}", category, do_text));
        }
        if !dont_text.is_empty() {
            components.push(format!("Avoid: {}", dont_text));
        }
    }

    if let Some(landing) = landing_results.first() {
        if let Some(sections) = landing.get("Section Order") {
            if !sections.is_empty() {
                layout.insert("Sections".to_string(), sections.clone());
            }
        }
        if let Some(cta) = landing.get("Primary CTA Placement") {
            if !cta.is_empty() {
                recommendations.push(format!("CTA Placement: {}", cta));
            }
        }
        if let Some(strategy) = landing.get("Color Strategy") {
            if !strategy.is_empty() {
                colors.insert("Strategy".to_string(), strategy.clone());
            }
        }
    }

    if layout.is_empty() {
        layout.insert("Max Width".to_string(), "1200px".to_string());
        layout.insert("Layout".to_string(), "Responsive grid".to_string());
    }

    if recommendations.is_empty() {
        recommendations.push("Refer to MASTER.md for all design rules".to_string());
        recommendations.push("Add specific overrides as needed for this page".to_string());
    }

    let mut output = HashMap::new();
    output.insert("page_type".to_string(), Value::String(page_type));
    output.insert("layout".to_string(), serde_json::to_value(layout).unwrap_or(Value::Null));
    output.insert("spacing".to_string(), serde_json::to_value(spacing).unwrap_or(Value::Null));
    output.insert("typography".to_string(), serde_json::to_value(typography).unwrap_or(Value::Null));
    output.insert("colors".to_string(), serde_json::to_value(colors).unwrap_or(Value::Null));
    output.insert("components".to_string(), serde_json::to_value(components).unwrap_or(Value::Null));
    output.insert("unique_components".to_string(), serde_json::to_value(unique_components).unwrap_or(Value::Null));
    output.insert("recommendations".to_string(), serde_json::to_value(recommendations).unwrap_or(Value::Null));
    output
}

fn format_page_override_md(design_system: &DesignSystem, page_name: &str, page_query: Option<&str>) -> String {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let page_title = page_name.replace(['-', '_'], " ").split_whitespace().map(|s| {
        let mut chars = s.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ");

    let overrides = generate_intelligent_overrides(page_name, page_query);

    let mut lines = Vec::new();
    lines.push(format!("# {} Page Overrides", page_title));
    lines.push(String::new());
    lines.push(format!("> **PROJECT:** {}", design_system.project_name));
    lines.push(format!("> **Generated:** {}", timestamp));
    lines.push(format!(
        "> **Page Type:** {}",
        overrides
            .get("page_type")
            .and_then(|v| v.as_str())
            .unwrap_or("General")
    ));
    lines.push(String::new());
    lines.push("> ⚠️ **IMPORTANT:** Rules in this file **override** the Master file (`design-system/MASTER.md`).".to_string());
    lines.push("> Only deviations from the Master are documented here. For all other rules, refer to the Master.".to_string());
    lines.push(String::new());
    lines.push("---".to_string());
    lines.push(String::new());

    lines.push("## Page-Specific Rules".to_string());
    lines.push(String::new());

    // Layout Overrides
    lines.push("### Layout Overrides".to_string());
    lines.push(String::new());
    if let Some(layout) = overrides.get("layout").and_then(|v| v.as_object()) {
        if layout.is_empty() {
            lines.push("- No overrides — use Master layout".to_string());
        } else {
            for (key, value) in layout {
                lines.push(format!("- **{}:** {}", key, value.as_str().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());

    // Spacing Overrides
    lines.push("### Spacing Overrides".to_string());
    lines.push(String::new());
    if let Some(spacing) = overrides.get("spacing").and_then(|v| v.as_object()) {
        if spacing.is_empty() {
            lines.push("- No overrides — use Master spacing".to_string());
        } else {
            for (key, value) in spacing {
                lines.push(format!("- **{}:** {}", key, value.as_str().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());

    // Typography Overrides
    lines.push("### Typography Overrides".to_string());
    lines.push(String::new());
    if let Some(typography) = overrides.get("typography").and_then(|v| v.as_object()) {
        if typography.is_empty() {
            lines.push("- No overrides — use Master typography".to_string());
        } else {
            for (key, value) in typography {
                lines.push(format!("- **{}:** {}", key, value.as_str().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());

    // Color Overrides
    lines.push("### Color Overrides".to_string());
    lines.push(String::new());
    if let Some(colors) = overrides.get("colors").and_then(|v| v.as_object()) {
        if colors.is_empty() {
            lines.push("- No overrides — use Master colors".to_string());
        } else {
            for (key, value) in colors {
                lines.push(format!("- **{}:** {}", key, value.as_str().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());

    // Components Overrides
    lines.push("### Component Overrides".to_string());
    lines.push(String::new());
    if let Some(components) = overrides.get("components").and_then(|v| v.as_array()) {
        if components.is_empty() {
            lines.push("- No overrides — use Master component specs".to_string());
        } else {
            for comp in components {
                lines.push(format!("- {}", comp.as_str().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("## Page-Specific Components".to_string());
    lines.push(String::new());
    if let Some(components) = overrides.get("unique_components").and_then(|v| v.as_array()) {
        if components.is_empty() {
            lines.push("- No unique components for this page".to_string());
        } else {
            for comp in components {
                lines.push(format!("- {}", comp.as_str().unwrap_or("")));
            }
        }
    }
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("## Recommendations".to_string());
    lines.push(String::new());
    if let Some(recommendations) = overrides.get("recommendations").and_then(|v| v.as_array()) {
        for rec in recommendations {
            lines.push(format!("- {}", rec.as_str().unwrap_or("")));
        }
    }
    lines.push(String::new());

    lines.join("\n")
}

fn persist_design_system(
    design_system: &DesignSystem,
    page: Option<&str>,
    output_dir: Option<&Path>,
    page_query: Option<&str>,
) -> Result<PersistSummary, String> {
    let base_dir = output_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // 关键路径使用净化结果防止路径穿越
    let project_slug = sanitize_slug(&design_system.project_name);

    let design_system_dir = base_dir.join("design-system").join(&project_slug);
    let pages_dir = design_system_dir.join("pages");

    std::fs::create_dir_all(&pages_dir)
        .map_err(|e| format!("创建目录失败: {}", e))?;

    let master_file = design_system_dir.join("MASTER.md");
    std::fs::write(&master_file, format_master_md(design_system))
        .map_err(|e| format!("写入 MASTER.md 失败: {}", e))?;

    let mut created = vec![master_file];

    if let Some(page_name) = page {
        let page_slug = sanitize_path_segment(page_name);
        let page_file = pages_dir.join(format!("{}.md", page_slug));
        std::fs::write(&page_file, format_page_override_md(design_system, page_name, page_query))
            .map_err(|e| format!("写入页面覆盖文件失败: {}", e))?;
        created.push(page_file);
    }

    Ok(PersistSummary {
        design_system_dir: design_system_dir.to_string_lossy().to_string(),
        created_files: created
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
    })
}

pub fn generate_design_system(
    query: &str,
    project_name: Option<&str>,
    format: Option<&str>,
    persist: bool,
    page: Option<&str>,
    output_dir: Option<&Path>,
) -> Result<DesignSystemOutput, String> {
    let generator = DesignSystemGenerator::new();
    let design_system = generator.generate(query, project_name);

    let persisted = if persist {
        Some(persist_design_system(&design_system, page, output_dir, Some(query))?)
    } else {
        None
    };

    let mut formatted = match normalize_format(format, "ascii").as_str() {
        "markdown" => format_markdown(&design_system),
        _ => format_ascii_box(&design_system),
    };

    if persist {
        let project_slug = sanitize_slug(&design_system.project_name);
        formatted.push_str("\n");
        formatted.push_str(&"=".repeat(60));
        formatted.push_str("\n");
        formatted.push_str(&format!(
            "✅ Design system persisted to design-system/{}/\n",
            project_slug
        ));
        formatted.push_str(&format!(
            "   📄 design-system/{}/MASTER.md (Global Source of Truth)\n",
            project_slug
        ));
        if let Some(page_name) = page {
            let page_slug = sanitize_path_segment(page_name);
            formatted.push_str(&format!(
                "   📄 design-system/{}/pages/{}.md (Page Overrides)\n",
                project_slug, page_slug
            ));
        }
        formatted.push_str("\n");
        formatted.push_str(&format!(
            "📖 Usage: When building a page, check design-system/{}/pages/[page].md first.\n",
            project_slug
        ));
        formatted.push_str("   If exists, its rules override MASTER.md. Otherwise, use MASTER.md.\n");
        formatted.push_str(&"=".repeat(60));
    }

    Ok(DesignSystemOutput {
        design_system,
        persisted,
        formatted,
    })
}

pub fn suggest(text: &str) -> SuggestResult {
    let store = &*UIUX_STORE;
    let mut matched: HashSet<String> = HashSet::new();

    // 1) 基础分词（允许 ui/ux 等短信号 + 处理中英粘连）
    for token in tokenize_for_suggest(text) {
        if store.keyword_set.contains(&token) {
            matched.insert(token);
        }
    }

    // 2) 中文强触发词：直接作为命中关键词（用于可解释性与兜底触发）
    for &kw in lexicon::ZH_UIUX_STRONG_TRIGGERS {
        if text.contains(kw) {
            matched.insert(kw.to_string());
        }
    }

    // 3) Query Expansion：将中文/同义概念映射为英文 token，再尝试命中 keyword_set
    for token in collect_query_expansion(text) {
        if store.keyword_set.contains(&token) {
            matched.insert(token);
        }
    }

    let mut matched_keywords: Vec<String> = matched.into_iter().collect();
    matched_keywords.sort();

    let score = matched_keywords.len();
    SuggestResult {
        should_suggest: score > 0,
        score,
        matched_keywords,
    }
}

fn build_keyword_set() -> HashSet<String> {
    let mut set = HashSet::new();

    let sources = [
        ("styles.csv", &["Style Category", "Keywords", "Best For", "Type"][..]),
        ("products.csv", &["Product Type", "Keywords", "Primary Style Recommendation", "Secondary Styles", "Landing Page Pattern"][..]),
        ("landing.csv", &["Pattern Name", "Keywords", "Section Order"][..]),
        ("ux-guidelines.csv", &["Category", "Issue", "Description"][..]),
        ("colors.csv", &["Product Type", "Keywords"][..]),
        ("typography.csv", &["Font Pairing Name", "Mood/Style Keywords", "Best For"][..]),
        ("charts.csv", &["Data Type", "Keywords", "Best Chart Type"][..]),
        ("ui-reasoning.csv", &["UI_Category", "Recommended_Pattern", "Style_Priority"][..]),
    ];

    for (file, columns) in sources {
        if let Ok(rows) = load_csv(file) {
            for row in rows {
                for col in columns {
                    if let Some(value) = row.get(*col) {
                        add_keywords(value, &mut set);
                    }
                }
            }
        }
    }

    // 栈/域名也走同一套清理规则，避免 `nuxt-ui` / `react-native` 这类 token 在 suggest 分词中无法命中
    for stack in STACK_CONFIGS.keys() {
        add_keywords_allow_short(stack, &mut set);
    }

    for domain in DOMAIN_CONFIGS.keys() {
        add_keywords_allow_short(domain, &mut set);
    }

    for token in [
        "ui",
        "ux",
        "uiux",
        "front-end",
        "frontend",
        "landing",
        "dashboard",
        "design",
        "component",
    ] {
        add_keywords_allow_short(token, &mut set);
    }

    set
}

fn add_keywords(text: &str, set: &mut HashSet<String>) {
    let lower = text.to_lowercase();
    let cleaned = TOKEN_RE.replace_all(&lower, " ");
    for token in cleaned.split_whitespace() {
        if token.len() <= 2 {
            continue;
        }
        if KEYWORD_STOPWORDS.contains(token) {
            continue;
        }
        set.insert(token.to_string());
    }
}

/// `build_keyword_set()` 用的“允许短 token”版本：保留长度 >= 2 的 token，支持 `ui/ux`。
fn add_keywords_allow_short(text: &str, set: &mut HashSet<String>) {
    let lower = text.to_lowercase();
    let cleaned = TOKEN_RE.replace_all(&lower, " ");
    for token in cleaned.split_whitespace() {
        if token.len() < 2 {
            continue;
        }
        if KEYWORD_STOPWORDS.contains(token) {
            continue;
        }
        set.insert(token.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_index() -> DomainIndex {
        static OUTPUT_COLS: &[&str] = &["Style Category"];

        let mut rows: Vec<HashMap<String, String>> = Vec::new();

        let mut r1 = HashMap::new();
        r1.insert("Style Category".to_string(), "Glassmorphism".to_string());
        r1.insert("Keywords".to_string(), "frosted glass blur".to_string());
        rows.push(r1);

        let mut r2 = HashMap::new();
        r2.insert("Style Category".to_string(), "Minimalism".to_string());
        r2.insert("Keywords".to_string(), "clean minimal elegant".to_string());
        rows.push(r2);

        let documents: Vec<String> = rows
            .iter()
            .map(|row| {
                format!(
                    "{} {}",
                    row.get("Style Category").cloned().unwrap_or_default(),
                    row.get("Keywords").cloned().unwrap_or_default()
                )
            })
            .collect();

        let mut bm25 = BM25::new(1.5, 0.75);
        bm25.fit(&documents);

        DomainIndex {
            file: "test.csv",
            output_cols: OUTPUT_COLS,
            rows,
            bm25,
            documents,
        }
    }

    #[test]
    fn tokenize_for_suggest_keeps_uiux_and_mixed_tokens() {
        let t1 = tokenize_for_suggest("UI/UX");
        assert!(t1.contains(&"ui".to_string()));
        assert!(t1.contains(&"ux".to_string()));

        // 中英粘连：应能提取出 ui
        let t2 = tokenize_for_suggest("UI美化");
        assert!(t2.contains(&"ui".to_string()));
    }

    #[test]
    fn query_expansion_maps_zh_to_en_tokens() {
        let extra = collect_query_expansion("科技感 登录");
        assert!(extra.iter().any(|t| t == "futuristic"));
        assert!(extra.iter().any(|t| t == "login"));
    }

    #[test]
    fn query_expansion_maps_ui_signal_to_long_tokens() {
        let extra = collect_query_expansion("UI美化");
        assert!(extra.iter().any(|t| t == "design"));
        assert!(extra.iter().any(|t| t == "interface"));
    }

    #[test]
    fn detect_domain_uses_zh_hints_first() {
        assert_eq!(detect_domain("配色方案"), "color");
        assert_eq!(detect_domain("字体 排版"), "typography");
        assert_eq!(detect_domain("图标 选择"), "icons");
    }

    #[test]
    fn fuzzy_fallback_recovers_from_typo() {
        let index = make_test_index();
        let results = index.search("glasomorphism", 1);
        assert!(!results.is_empty());
        assert_eq!(
            results[0].get("Style Category").map(|s| s.as_str()),
            Some("Glassmorphism")
        );
    }

    #[test]
    fn suggest_triggers_on_common_zh_intent() {
        let r = suggest("优雅的登录页面，帮我美化一下");
        assert!(r.should_suggest);
        assert!(r.matched_keywords.iter().any(|k| k == "登录" || k == "美化"));
    }
}
