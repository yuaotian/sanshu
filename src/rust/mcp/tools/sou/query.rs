use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchIntent {
    #[default]
    Auto,
    Code,
    Docs,
}

#[derive(Debug, Clone)]
pub(super) struct QueryPlan {
    pub query: String,
    pub terms: Vec<String>,
    pub files: Vec<String>,
    pub identifiers: Vec<String>,
    pub intent: SearchIntent,
    pub cross_module: bool,
    pub terms_truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Relevance {
    pub file_match: bool,
    pub definition_hits: usize,
    pub identifier_hits: usize,
    pub weighted_coverage: usize,
}

pub(super) fn is_document(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|ext| ext.to_str()),
        Some("md" | "mdx" | "txt" | "rst" | "adoc")
    )
}

fn raw_words(query: &str) -> impl Iterator<Item = &str> {
    query
        .split(|ch: char| {
            !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | '.' | '/' | '\\' | ':')
        })
        .filter(|word| !word.is_empty())
}

pub(super) fn file_references(query: &str) -> Vec<String> {
    let mut files = Vec::new();
    for word in raw_words(query) {
        let normalized = word.replace('\\', "/");
        let trimmed = normalized.trim_end_matches(['.', ':']).to_string();
        let extension = Path::new(&trimmed).extension().and_then(|ext| ext.to_str());
        // 中文说明：仅把已支持的文件后缀视为路径，避免将 A::B、版本号当作文件名。
        if extension.is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "rs" | "c"
                    | "cc"
                    | "cpp"
                    | "cxx"
                    | "h"
                    | "hpp"
                    | "cs"
                    | "go"
                    | "java"
                    | "kt"
                    | "swift"
                    | "py"
                    | "rb"
                    | "php"
                    | "lua"
                    | "js"
                    | "mjs"
                    | "ts"
                    | "tsx"
                    | "jsx"
                    | "vue"
                    | "svelte"
                    | "html"
                    | "css"
                    | "scss"
                    | "sql"
                    | "proto"
                    | "json"
                    | "jsonc"
                    | "yaml"
                    | "yml"
                    | "toml"
                    | "xml"
                    | "md"
                    | "mdx"
                    | "txt"
                    | "rst"
                    | "adoc"
                    | "sh"
                    | "ps1"
                    | "bat"
            )
        }) && !files.contains(&trimmed)
        {
            files.push(trimmed);
        }
    }
    files
}

impl QueryPlan {
    pub fn new(query: &str, preference: SearchIntent) -> Self {
        let files = file_references(query);
        let mut content_query = query.to_string();
        for file in &files {
            content_query = content_query
                .replace(file, " ")
                .replace(&file.replace('/', "\\"), " ");
        }
        let identifiers = raw_words(&content_query)
            .filter(|word| word.is_ascii() && word.chars().any(|ch| ch.is_ascii_alphabetic()))
            .filter(|word| {
                word.contains('_')
                    || word.contains("::")
                    || word.chars().skip(1).any(|ch| ch.is_ascii_uppercase())
                    || content_query.trim() == *word
            })
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let stopwords = [
            "the", "and", "for", "from", "with", "this", "that", "what", "where", "when", "代码",
            "项目", "搜索", "相关", "实现", "如何", "怎么", "什么", "是否",
        ];
        let mut terms = super::local::tokenize_text(&content_query, usize::MAX)
            .into_iter()
            .filter(|term| term.chars().count() >= 2 && !stopwords.contains(&term.as_str()))
            .collect::<Vec<_>>();
        terms.sort_by_key(|term| {
            if identifiers.contains(term) {
                0
            } else if term.is_ascii() {
                1
            } else if (2..=3).contains(&term.chars().count()) {
                2
            } else {
                3
            }
        });
        // 中文说明：文件名走独立召回，日期和后缀不再挤占正文关键词预算。
        if terms.is_empty() {
            if !query.trim().is_empty() && query.trim().chars().all(|ch| ch.is_ascii_digit()) {
                terms.push(query.trim().to_string());
            }
            terms.extend(
                files
                    .iter()
                    .filter_map(|file| Path::new(file).file_stem())
                    .map(|stem| stem.to_string_lossy().to_lowercase()),
            );
        }
        let terms_truncated = terms.len() > 24;
        terms.truncate(24);
        let lower = query.to_lowercase();
        let intent = match preference {
            SearchIntent::Auto
                if files.iter().any(|file| is_document(&file.to_lowercase()))
                    || (files.is_empty()
                        && [
                            "文档",
                            "实施计划",
                            "验收",
                            "设计方案",
                            "readme",
                            "documentation",
                        ]
                        .iter()
                        .any(|term| lower.contains(term))) =>
            {
                SearchIntent::Docs
            }
            SearchIntent::Auto => SearchIntent::Code,
            explicit => explicit,
        };
        Self {
            query: lower,
            terms,
            files,
            identifiers,
            intent,
            terms_truncated,
            cross_module: [
                "调用链",
                "跨模块",
                "端到端",
                "生命周期",
                "call chain",
                "end-to-end",
            ]
            .iter()
            .any(|term| query.to_lowercase().contains(term)),
        }
    }

    pub fn matches_file(&self, path: &str) -> bool {
        let path = path.replace('\\', "/").to_lowercase();
        self.files.iter().any(|file| {
            let file = file.replace('\\', "/").to_lowercase();
            path == file || path.ends_with(&format!("/{file}"))
        })
    }

    pub fn relevance(&self, path: &str, content: &str) -> Relevance {
        let lower = content.to_lowercase();
        let path = path.replace('\\', "/").to_lowercase();
        let basename = path.rsplit('/').next().unwrap_or(&path);
        let identifier_hits = self
            .identifiers
            .iter()
            .filter(|id| contains_identifier(&lower, id) || contains_identifier(basename, id))
            .count();
        let coverage = self
            .terms
            .iter()
            .filter(|term| lower.contains(term.as_str()))
            .count();
        // 中文说明：声明优先于测试数据中的查询字符串，避免回归样例污染检索首位。
        let definition_hits = if self.intent == SearchIntent::Code && !is_document(&path) {
            self.identifiers
                .iter()
                .filter(|identifier| {
                    lower.lines().any(|line| {
                        let line = line
                            .strip_prefix('L')
                            .and_then(|numbered| numbered.split_once(':'))
                            .filter(|(number, _)| {
                                !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
                            })
                            .map(|(_, content)| content)
                            .unwrap_or(line);
                        line.match_indices(identifier.as_str()).any(|(offset, _)| {
                            let prefix = line[..offset].trim();
                            !prefix.contains(['\"', '\'', '`'])
                                && !prefix.starts_with("//")
                                && matches!(
                                    prefix.split_whitespace().next_back(),
                                    Some(
                                        "fn" | "struct"
                                            | "enum"
                                            | "impl"
                                            | "class"
                                            | "interface"
                                            | "type"
                                            | "def"
                                            | "function"
                                            | "const"
                                    )
                                )
                                && contains_identifier(line, identifier)
                        })
                    })
                })
                .count()
        } else {
            0
        };
        let path_coverage = self
            .terms
            .iter()
            .filter(|term| basename.contains(term.as_str()))
            .count();
        let preferred = match self.intent {
            SearchIntent::Docs => is_document(&path),
            _ => !is_document(&path),
        };
        Relevance {
            file_match: self.matches_file(&path),
            definition_hits,
            identifier_hits,
            // 中文说明：类型仅提供有界加分；精确文件与完整标识符始终先于类型偏好。
            weighted_coverage: coverage * 4
                + path_coverage.min(3) * 2
                + usize::from(preferred && (coverage > 0 || path_coverage > 0)) * 6,
        }
    }
}

fn contains_identifier(text: &str, identifier: &str) -> bool {
    text.match_indices(identifier).any(|(offset, _)| {
        let before = text[..offset].chars().next_back();
        let after = text[offset + identifier.len()..].chars().next();
        let boundary = |ch: Option<char>| ch.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_');
        boundary(before) && boundary(after)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_dates_do_not_consume_content_terms() {
        let plan = QueryPlan::new(
            "find-mouse-theme-2.5d-plan-2026-09-16.md 万剑归宗 实施计划",
            SearchIntent::Auto,
        );
        assert_eq!(plan.intent, SearchIntent::Docs);
        assert!(!plan
            .terms
            .iter()
            .any(|term| ["2026", "09", "16", "md"].contains(&term.as_str())));
        assert!(plan.matches_file("docs/find-mouse-theme-2.5d-plan-2026-09-16.md"));
    }

    #[test]
    fn exact_identifier_beats_incidental_words_and_numeric_queries_survive() {
        let plan = QueryPlan::new("ensure_watcher 文件监听", SearchIntent::Auto);
        assert!(
            plan.relevance("src/local.rs", "fn ensure_watcher() {}")
                > plan.relevance("docs/readme.md", "文件监听 watcher 使用说明")
        );
        assert_eq!(QueryPlan::new("160", SearchIntent::Auto).terms, vec!["160"]);
        assert_eq!(QueryPlan::new("1", SearchIntent::Auto).terms, vec!["1"]);
    }
}
