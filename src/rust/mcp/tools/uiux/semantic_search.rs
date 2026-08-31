//! UIUX 本地混合检索：结构化 BM25 保底，BGE 语义结果用加权 RRF 融合。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::model_manager;
use super::structured_search::{self, KnowledgeHit, SearchReport};
use super::types::UiuxAction;

const BM25_WEIGHT: f64 = 0.65;
const SEMANTIC_WEIGHT: f64 = 0.35;
const RRF_OFFSET: f64 = 60.0;
const SEMANTIC_CANDIDATE_LIMIT: usize = 48;
pub(super) const SEMANTIC_ONLY_MIN_SCORE: f32 = 0.46;
const MODEL_WAIT_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct HybridSearchOutcome {
    pub report: SearchReport,
    pub engine: String,
    pub semantic_state: String,
    pub semantic_top_score: Option<f32>,
    pub fusion: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct FusedCandidate {
    identity: String,
    domain: String,
    hit: KnowledgeHit,
    lexical_rank: Option<usize>,
    semantic_rank: Option<usize>,
    semantic_score: Option<f32>,
}

impl FusedCandidate {
    fn score(&self) -> f64 {
        let lexical = self
            .lexical_rank
            .map(|rank| BM25_WEIGHT / (RRF_OFFSET + rank as f64))
            .unwrap_or_default();
        let semantic = self
            .semantic_rank
            .map(|rank| SEMANTIC_WEIGHT / (RRF_OFFSET + rank as f64))
            .unwrap_or_default();
        lexical + semantic
    }
}

pub async fn search(query: &str, action: UiuxAction, max_results: usize) -> HybridSearchOutcome {
    let bm25 = structured_search::search(query, action, max_results.max(8));
    let settings = model_manager::semantic_settings();
    if !settings.enabled {
        return bm25_only(
            bm25,
            "disabled",
            "UIUX 本地语义检索已关闭，auto 使用结构化 BM25",
        );
    }

    let ranking =
        match model_manager::rank_documents(query, &settings.model_dir, MODEL_WAIT_BUDGET).await {
            Ok(value) => value,
            Err(unavailable) => {
                return bm25_only(bm25, &unavailable.state, &unavailable.message);
            }
        };
    let semantic_top_score = ranking.top_score;
    if bm25.abstained && semantic_top_score < SEMANTIC_ONLY_MIN_SCORE {
        return HybridSearchOutcome {
            report: bm25,
            engine: "local_bm25_bge_rrf_v1".to_string(),
            semantic_state: "ready".to_string(),
            semantic_top_score: Some(semantic_top_score),
            fusion: None,
            message: format!(
                "BGE 语义最高分 {:.4} 低于纯语义召回阈值 {:.2}",
                semantic_top_score, SEMANTIC_ONLY_MIN_SCORE
            ),
        };
    }
    let report = fuse_reports(bm25, &ranking.matches, max_results);
    HybridSearchOutcome {
        report,
        engine: "local_bm25_bge_rrf_v1".to_string(),
        semantic_state: "ready".to_string(),
        semantic_top_score: Some(semantic_top_score),
        fusion: Some(format!(
            "weighted_rrf(bm25={BM25_WEIGHT:.2},semantic={SEMANTIC_WEIGHT:.2},k={RRF_OFFSET:.0})"
        )),
        message: "auto 已使用 UIUX 本地 BM25 + BGE 语义混合检索".to_string(),
    }
}

fn bm25_only(bm25: SearchReport, semantic_state: &str, message: &str) -> HybridSearchOutcome {
    HybridSearchOutcome {
        report: bm25,
        engine: structured_search::KNOWLEDGE_ENGINE.to_string(),
        semantic_state: semantic_state.to_string(),
        semantic_top_score: None,
        fusion: None,
        message: message.to_string(),
    }
}

fn fuse_reports(
    bm25: SearchReport,
    semantic_matches: &[model_manager::SemanticMatch],
    max_results: usize,
) -> SearchReport {
    let documents = structured_search::semantic_documents();
    let mut candidates = HashMap::<String, FusedCandidate>::new();

    for (rank, hit) in bm25.hits.iter().enumerate() {
        if let Some(document) = documents
            .iter()
            .find(|document| document.location == hit.location)
        {
            candidates.insert(
                hit.location.clone(),
                FusedCandidate {
                    identity: normalize_identity(&document.identity),
                    domain: document.domain.clone(),
                    hit: hit.clone(),
                    lexical_rank: Some(rank + 1),
                    semantic_rank: None,
                    semantic_score: None,
                },
            );
        }
    }

    for (rank, semantic_match) in semantic_matches
        .iter()
        .take(SEMANTIC_CANDIDATE_LIMIT)
        .enumerate()
    {
        let Some(document) = documents.get(semantic_match.document_index) else {
            continue;
        };
        let candidate = candidates
            .entry(document.location.clone())
            .or_insert_with(|| FusedCandidate {
                identity: normalize_identity(&document.identity),
                domain: document.domain.clone(),
                hit: KnowledgeHit {
                    source: "local_hybrid".to_string(),
                    location: document.location.clone(),
                    excerpt: document.excerpt.clone(),
                    domain: document.domain.clone(),
                },
                lexical_rank: None,
                semantic_rank: None,
                semantic_score: None,
            });
        candidate.semantic_rank = Some(rank + 1);
        candidate.semantic_score = Some(semantic_match.score);
        candidate.hit.source = "local_hybrid".to_string();
    }

    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score()
            .total_cmp(&left.score())
            .then_with(|| {
                right
                    .semantic_score
                    .unwrap_or(-1.0)
                    .total_cmp(&left.semantic_score.unwrap_or(-1.0))
            })
            .then_with(|| left.hit.location.cmp(&right.hit.location))
    });
    let hits = select_diverse(candidates, max_results.clamp(1, 8));
    let domains = ordered_unique(hits.iter().map(|hit| hit.domain.clone()));

    SearchReport {
        abstained: hits.is_empty(),
        hits,
        rewritten_query: bm25.rewritten_query,
        query_rewrites: bm25.query_rewrites,
        domains,
        top_score: bm25.top_score,
        token_coverage: bm25.token_coverage,
    }
}

fn select_diverse(candidates: Vec<FusedCandidate>, limit: usize) -> Vec<KnowledgeHit> {
    let mut selected = Vec::new();
    let mut identities = HashSet::new();
    let mut domains = HashSet::new();

    // 与 BM25 主链一致：第一轮跨域取样，第二轮再允许同域补位。
    for candidate in &candidates {
        if selected.len() >= limit {
            break;
        }
        if domains.contains(candidate.domain.as_str())
            || identities.contains(candidate.identity.as_str())
        {
            continue;
        }
        domains.insert(candidate.domain.clone());
        identities.insert(candidate.identity.clone());
        selected.push(candidate.hit.clone());
    }
    if selected.len() < limit {
        for candidate in candidates {
            if selected.len() >= limit {
                break;
            }
            if identities.insert(candidate.identity) {
                selected.push(candidate.hit);
            }
        }
    }
    selected
}

fn normalize_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn ordered_unique(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values.filter(|value| seen.insert(value.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_keeps_lexical_exact_match_ahead_of_semantic_only_match() {
        let lexical = FusedCandidate {
            identity: "exact".to_string(),
            domain: "styles".to_string(),
            hit: KnowledgeHit {
                source: "local_bm25".to_string(),
                location: "exact".to_string(),
                excerpt: String::new(),
                domain: "styles".to_string(),
            },
            lexical_rank: Some(1),
            semantic_rank: None,
            semantic_score: None,
        };
        let semantic = FusedCandidate {
            semantic_rank: Some(1),
            lexical_rank: None,
            ..lexical.clone()
        };
        assert!(lexical.score() > semantic.score());
    }

    #[test]
    fn semantic_only_threshold_keeps_measured_signal_boundary() {
        assert!(0.4783 >= SEMANTIC_ONLY_MIN_SCORE);
        assert!(0.4417 < SEMANTIC_ONLY_MIN_SCORE);
    }
}
