use anyhow::{anyhow, Context, Result};
use ignore::WalkBuilder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use once_cell::sync::Lazy;
use ring::digest::{Context as ShaContext, SHA256};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;

use super::{reranker, semantic};

const INDEX_MISSING: u8 = 0;
const INDEX_BUILDING: u8 = 1;
const INDEX_READY: u8 = 2;
const INDEX_ERROR: u8 = 3;
const SEMANTIC_DISABLED: u8 = 0;
const SEMANTIC_MISSING: u8 = 1;
const SEMANTIC_BUILDING: u8 = 2;
const SEMANTIC_SYNCING: u8 = 3;
const SEMANTIC_READY: u8 = 4;
const SEMANTIC_ERROR: u8 = 5;
const CHUNK_LINES: usize = 80;
const CHUNK_OVERLAP: usize = 20;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_QUERY_TERMS: usize = 24;
const ACCURATE_LEXICAL_LIMIT: usize = 10;
const ACCURATE_SEMANTIC_LIMIT: usize = 50;
const ACCURATE_CANDIDATE_LIMIT: usize = 60;
const ACCURATE_RERANK_LIMIT: usize = 32;
const ACCURATE_QUERY_BUDGET: Duration = Duration::from_secs(3);
const ACCURATE_FUSION_NAME: &str = "bge_reranker_base_top50_top10_file32_rrf";
const RERANKER_CONTEXT_CHARS: usize = 768;
const RERANKER_RRF_WEIGHT: f64 = 0.70;
const RETRIEVAL_RRF_WEIGHT: f64 = 0.30;

static PROJECT_INDEXES: Lazy<Mutex<HashMap<PathBuf, Arc<ProjectIndex>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub(super) struct LocalSearchOptions {
    pub project_root: PathBuf,
    pub query: String,
    pub max_results: usize,
    pub exclude_paths: Vec<String>,
    pub index_dir: PathBuf,
    pub semantic: LocalSemanticSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalSemanticMode {
    Off,
    Balanced,
    Accurate,
}

impl LocalSemanticMode {
    pub(crate) fn from_effective(value: &str) -> Self {
        match value {
            crate::config::SOU_SEMANTIC_MODE_ACCURATE => Self::Accurate,
            crate::config::SOU_SEMANTIC_MODE_BALANCED => Self::Balanced,
            _ => Self::Off,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => crate::config::SOU_SEMANTIC_MODE_OFF,
            Self::Balanced => crate::config::SOU_SEMANTIC_MODE_BALANCED,
            Self::Accurate => crate::config::SOU_SEMANTIC_MODE_ACCURATE,
        }
    }

    pub(crate) fn enabled(self) -> bool {
        self != Self::Off
    }

    pub(crate) fn accurate(self) -> bool {
        self == Self::Accurate
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LocalSemanticSettings {
    pub mode: LocalSemanticMode,
    pub model_dir: PathBuf,
    pub reranker_model_dir: PathBuf,
}

impl LocalSemanticSettings {
    fn enabled(&self) -> bool {
        self.mode.enabled()
    }
}

#[derive(Debug, Clone)]
pub(super) struct LocalSearchOutput {
    pub text: String,
    pub hit_count: usize,
    pub engine: String,
    pub index_state: String,
    pub degraded: bool,
    pub fallback_reason: Option<String>,
    pub notice: Option<String>,
    pub duration_ms: u64,
    pub semantic_state: String,
    pub semantic_model: Option<String>,
    pub semantic_indexed_chunks: u64,
    pub semantic_pending_chunks: u64,
    pub semantic_top_score: Option<f32>,
    pub semantic_mode: String,
    pub reranker_state: Option<String>,
    pub reranker_model: Option<String>,
    pub reranker_duration_ms: Option<u64>,
    pub reranker_top_score: Option<f32>,
    #[cfg(test)]
    pub reranker_input_diagnostics: Option<RerankerInputDiagnostics>,
    pub fusion: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
pub(super) struct RerankerInputDiagnostics {
    pub candidate_count: usize,
    pub batch_count: usize,
    pub truncated_candidate_count: usize,
    pub total_chars: usize,
    pub min_chars: usize,
    pub p50_chars: usize,
    pub p95_chars: usize,
    pub max_chars: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalIndexScopeStatus {
    pub name: String,
    pub relative_path: String,
    pub project_root: String,
    pub index_path: String,
    pub state: String,
    pub indexed_files: u64,
    pub indexed_chunks: u64,
    pub lexical_sync_running: bool,
    pub pending_changes: bool,
    pub semantic_state: String,
    pub semantic_indexed_chunks: u64,
    pub semantic_pending_chunks: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalIndexStatus {
    pub is_workspace: bool,
    pub project_count: usize,
    pub scopes: Vec<LocalIndexScopeStatus>,
    pub project_root: String,
    pub index_path: String,
    pub state: String,
    pub indexed_files: u64,
    pub indexed_chunks: u64,
    pub sync_running: bool,
    pub lexical_sync_running: bool,
    pub semantic_sync_running: bool,
    pub pending_changes: bool,
    pub last_error: Option<String>,
    pub semantic_state: String,
    pub semantic_model: Option<String>,
    pub semantic_indexed_chunks: u64,
    pub semantic_pending_chunks: u64,
    pub semantic_last_error: Option<String>,
    pub semantic_requested_provider: String,
    pub semantic_execution_provider: String,
    pub semantic_provider_fallback_reason: Option<String>,
    pub semantic_cuda_runtime_available: bool,
    pub semantic_cuda_runtime_dir: Option<String>,
    pub semantic_cuda_runtime_error: Option<String>,
    pub semantic_batch_size: usize,
    pub semantic_intra_threads: Option<usize>,
}

#[derive(Debug, Clone)]
struct SearchHit {
    relative_path: String,
    start_line: usize,
    end_line: usize,
    excerpt: String,
    coverage: usize,
    exact_match: bool,
    path_matches: usize,
    lexical_score: f64,
    semantic_score: Option<f32>,
    fusion_score: f64,
}

struct ProjectIndex {
    root: PathBuf,
    db_path: PathBuf,
    state: AtomicU8,
    indexed_files: AtomicU64,
    indexed_chunks: AtomicU64,
    sync_running: AtomicBool,
    lexical_sync_running: AtomicBool,
    startup_reconcile_pending: AtomicBool,
    dirty: Arc<AtomicBool>,
    profile_hash: Mutex<String>,
    last_error: Mutex<Option<String>>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    semantic_state: AtomicU8,
    semantic_indexed_chunks: AtomicU64,
    semantic_pending_chunks: AtomicU64,
    semantic_last_error: Mutex<Option<String>>,
}

impl ProjectIndex {
    fn new(root: PathBuf, db_path: PathBuf) -> Self {
        let (state, files, chunks, error) = inspect_existing_index(&db_path);
        let semantic_stats = semantic::inspect(&db_path).ok();
        let semantic_state = match semantic_stats {
            Some(stats) if stats.pending_chunks == 0 && stats.indexed_chunks > 0 => SEMANTIC_READY,
            _ => SEMANTIC_MISSING,
        };
        Self {
            root,
            db_path,
            state: AtomicU8::new(state),
            indexed_files: AtomicU64::new(files),
            indexed_chunks: AtomicU64::new(chunks),
            sync_running: AtomicBool::new(false),
            lexical_sync_running: AtomicBool::new(false),
            startup_reconcile_pending: AtomicBool::new(state == INDEX_READY),
            dirty: Arc::new(AtomicBool::new(false)),
            profile_hash: Mutex::new(String::new()),
            last_error: Mutex::new(error),
            watcher: Mutex::new(None),
            semantic_state: AtomicU8::new(semantic_state),
            semantic_indexed_chunks: AtomicU64::new(
                semantic_stats
                    .map(|stats| stats.indexed_chunks)
                    .unwrap_or_default(),
            ),
            semantic_pending_chunks: AtomicU64::new(
                semantic_stats
                    .map(|stats| stats.pending_chunks)
                    .unwrap_or_default(),
            ),
            semantic_last_error: Mutex::new(None),
        }
    }

    fn state_name(&self) -> &'static str {
        match self.state.load(Ordering::Acquire) {
            INDEX_BUILDING => "building",
            INDEX_READY => "ready",
            INDEX_ERROR => "error",
            _ => "missing",
        }
    }

    fn ensure_watcher(&self) -> Result<()> {
        let mut guard = self
            .watcher
            .lock()
            .map_err(|_| anyhow!("本地索引 watcher 锁已损坏"))?;
        if guard.is_some() {
            return Ok(());
        }

        let dirty = Arc::clone(&self.dirty);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
                Ok(_) => {
                    dirty.store(true, Ordering::Release);
                }
                Err(error) => log::warn!("[sou-local] 文件监听事件失败: {}", error),
            })
            .context("创建本地索引文件监听器失败")?;
        watcher
            .watch(&self.root, RecursiveMode::Recursive)
            .with_context(|| format!("监听项目目录失败: {}", self.root.display()))?;
        *guard = Some(watcher);
        Ok(())
    }

    fn semantic_state_name(&self, enabled: bool) -> &'static str {
        if !enabled {
            return "disabled";
        }
        match self.semantic_state.load(Ordering::Acquire) {
            SEMANTIC_BUILDING => "building",
            SEMANTIC_SYNCING => "syncing",
            SEMANTIC_READY => "ready",
            SEMANTIC_ERROR => "error",
            _ => "missing",
        }
    }

    fn status(&self, semantic_settings: &LocalSemanticSettings) -> LocalIndexStatus {
        let embedding_snapshot = crate::mcp::embedding::snapshot(&semantic_settings.model_dir);
        let semantic_state = self.semantic_state.load(Ordering::Acquire);
        LocalIndexStatus {
            is_workspace: false,
            project_count: 1,
            scopes: Vec::new(),
            project_root: normalize_path(&self.root),
            index_path: normalize_path(&self.db_path),
            state: self.state_name().to_string(),
            indexed_files: self.indexed_files.load(Ordering::Acquire),
            indexed_chunks: self.indexed_chunks.load(Ordering::Acquire),
            sync_running: self.sync_running.load(Ordering::Acquire),
            lexical_sync_running: self.lexical_sync_running.load(Ordering::Acquire),
            semantic_sync_running: matches!(semantic_state, SEMANTIC_BUILDING | SEMANTIC_SYNCING),
            pending_changes: self.dirty.load(Ordering::Acquire)
                || self.startup_reconcile_pending.load(Ordering::Acquire),
            last_error: self.last_error.lock().ok().and_then(|value| value.clone()),
            semantic_state: self
                .semantic_state_name(semantic_settings.enabled())
                .to_string(),
            semantic_model: semantic_settings.enabled().then(|| semantic::model_key()),
            semantic_indexed_chunks: self.semantic_indexed_chunks.load(Ordering::Acquire),
            semantic_pending_chunks: self.semantic_pending_chunks.load(Ordering::Acquire),
            semantic_last_error: self
                .semantic_last_error
                .lock()
                .ok()
                .and_then(|value| value.clone()),
            semantic_requested_provider: embedding_snapshot.requested_provider,
            semantic_execution_provider: embedding_snapshot.execution_provider,
            semantic_provider_fallback_reason: embedding_snapshot.provider_fallback_reason,
            semantic_cuda_runtime_available: embedding_snapshot.cuda_runtime_available,
            semantic_cuda_runtime_dir: embedding_snapshot
                .cuda_runtime_dir
                .as_deref()
                .map(normalize_path),
            semantic_cuda_runtime_error: embedding_snapshot.cuda_runtime_error,
            semantic_batch_size: embedding_snapshot.batch_size,
            semantic_intra_threads: embedding_snapshot.intra_threads,
        }
    }
}

pub(super) async fn search(options: LocalSearchOptions) -> Result<LocalSearchOutput> {
    let root = options
        .project_root
        .canonicalize()
        .with_context(|| format!("本地搜索项目路径无效: {}", options.project_root.display()))?;
    if !root.is_dir() {
        return Err(anyhow!("本地搜索项目路径不是目录: {}", root.display()));
    }

    let index = project_index(&root, &options.index_dir)?;
    search_with_index(options, root, index, true).await
}

#[cfg(test)]
pub(super) async fn search_for_test(
    options: LocalSearchOptions,
    index_path: PathBuf,
) -> Result<LocalSearchOutput> {
    let root = options
        .project_root
        .canonicalize()
        .with_context(|| format!("本地搜索项目路径无效: {}", options.project_root.display()))?;
    if !root.is_dir() {
        return Err(anyhow!("本地搜索项目路径不是目录: {}", root.display()));
    }

    let index = Arc::new(ProjectIndex::new(root.clone(), index_path));
    refresh_profile(&index, &options.exclude_paths);
    sync_now(
        Arc::clone(&index),
        options.exclude_paths.clone(),
        options.semantic.clone(),
    )
    .await?;
    search_with_index(options, root, index, false).await
}

async fn search_with_index(
    options: LocalSearchOptions,
    root: PathBuf,
    index: Arc<ProjectIndex>,
    enable_watcher: bool,
) -> Result<LocalSearchOutput> {
    let started_at = Instant::now();
    let semantic_mode = options.semantic.mode;
    if !semantic_mode.accurate() {
        reranker::release();
    }
    let terms = extract_query_terms(&options.query);
    if terms.is_empty() {
        return Err(anyhow!("本地搜索未提取到有效关键词"));
    }

    if enable_watcher {
        if let Err(error) = index.ensure_watcher() {
            log::warn!(
                "[sou-local] watcher 启动失败，继续使用查询时对账: {}",
                error
            );
        }
    }
    refresh_profile(&index, &options.exclude_paths);

    let mut degraded = false;
    let mut fallback_reason = None;
    let mut notice = None;
    let (mut hits, mut engine) = if index.state.load(Ordering::Acquire) == INDEX_READY {
        let dirty = index.dirty.load(Ordering::Acquire);
        let lexical_sync_running = index.lexical_sync_running.load(Ordering::Acquire);
        let startup_reconcile_pending = index.startup_reconcile_pending.load(Ordering::Acquire);
        if dirty || lexical_sync_running || startup_reconcile_pending {
            notice = Some(if dirty {
                "检测到文件变更，本次已使用即时搜索，后台词法索引正在同步".to_string()
            } else if startup_reconcile_pending {
                "进程启动后正在校验文件元数据，本次已使用即时搜索".to_string()
            } else {
                "后台词法索引正在同步，本次已使用即时搜索".to_string()
            });
            schedule_sync(
                Arc::clone(&index),
                options.exclude_paths.clone(),
                options.semantic.clone(),
            );
            run_immediate_search(&root, &options, &terms).await?
        } else {
            let db_path = index.db_path.clone();
            let query = options.query.clone();
            let query_terms = terms.clone();
            let max_results = if semantic_mode.accurate() {
                ACCURATE_LEXICAL_LIMIT
            } else if options.semantic.enabled() {
                options.max_results.saturating_mul(5).min(150)
            } else {
                options.max_results
            };
            match tokio::task::spawn_blocking(move || {
                query_index(&db_path, &query, &query_terms, max_results)
            })
            .await
            .context("等待 FTS5 查询任务失败")?
            {
                Ok(hits) => (hits, "fts5".to_string()),
                Err(error) => {
                    let reason = format!("FTS5 查询失败: {}", error);
                    degraded = true;
                    mark_index_error(&index, &reason);
                    schedule_sync(
                        Arc::clone(&index),
                        options.exclude_paths.clone(),
                        options.semantic.clone(),
                    );
                    fallback_reason = Some(reason);
                    run_immediate_search(&root, &options, &terms).await?
                }
            }
        }
    } else {
        let state = index.state_name().to_string();
        fallback_reason = Some(format!("本地索引状态为 {}", state));
        schedule_sync(
            Arc::clone(&index),
            options.exclude_paths.clone(),
            options.semantic.clone(),
        );
        run_immediate_search(&root, &options, &terms).await?
    };

    let mut semantic_top_score = None;
    let mut reranker_state = semantic_mode.accurate().then(|| "skipped".to_string());
    let reranker_model = semantic_mode
        .accurate()
        .then(|| reranker::MODEL_NAME.to_string());
    let mut reranker_duration_ms = None;
    let mut reranker_top_score = None;
    #[cfg(test)]
    let mut reranker_input_diagnostics = None;
    let mut fusion = None;
    if options.semantic.enabled() && engine == "fts5" {
        let semantic_deadline = if semantic_mode.accurate() {
            started_at + accurate_query_budget()
        } else {
            Instant::now() + Duration::from_secs(2)
        };
        if !crate::mcp::embedding::assets_available_for_provider(
            &options.semantic.model_dir,
            crate::mcp::embedding::configured_provider(),
        ) {
            index
                .semantic_state
                .store(SEMANTIC_MISSING, Ordering::Release);
            append_fallback(
                &mut fallback_reason,
                "BGE 模型资产未就绪，本次保留 FTS5 结果",
            );
        } else {
            if index.semantic_state.load(Ordering::Acquire) != SEMANTIC_READY {
                schedule_sync(
                    Arc::clone(&index),
                    options.exclude_paths.clone(),
                    options.semantic.clone(),
                );
                while matches!(
                    index.semantic_state.load(Ordering::Acquire),
                    SEMANTIC_BUILDING | SEMANTIC_SYNCING
                ) && Instant::now() < semantic_deadline
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }

            if index.semantic_state.load(Ordering::Acquire) == SEMANTIC_READY {
                let semantic_limit = if semantic_mode.accurate() {
                    ACCURATE_SEMANTIC_LIMIT
                } else {
                    options.max_results.saturating_mul(5).min(150)
                };
                let remaining_budget = semantic_deadline.saturating_duration_since(Instant::now());
                let semantic_result = if remaining_budget.is_zero() {
                    Err("loading: 语义查询等待预算已用尽".to_string())
                } else {
                    semantic::search(
                        &index.db_path,
                        &options.semantic.model_dir,
                        &options.query,
                        semantic_limit,
                        remaining_budget,
                    )
                    .await
                };
                match semantic_result {
                    Ok(semantic_hits) => {
                        semantic_top_score = semantic_hits.first().map(|hit| hit.score);
                        engine = "fts5+bge".to_string();
                        fusion = Some(semantic::FUSION_NAME.to_string());
                        if semantic_mode.accurate() {
                            let protected_exact =
                                hits.first().filter(|hit| hit.exact_match).cloned();
                            let retrieval_candidates = fuse_hits(
                                hits,
                                semantic_hits,
                                &options.query,
                                &terms,
                                ACCURATE_CANDIDATE_LIMIT,
                            );
                            hits = retrieval_candidates
                                .iter()
                                .take(options.max_results.max(1))
                                .cloned()
                                .collect();
                            let candidates =
                                select_accurate_rerank_candidates(retrieval_candidates);
                            if !reranker::assets_have_expected_sizes(
                                &options.semantic.reranker_model_dir,
                            ) {
                                reranker_state = Some("missing".to_string());
                                append_fallback(
                                    &mut fallback_reason,
                                    "准确模式模型资产未就绪，本次返回均衡模式结果",
                                );
                            } else {
                                reranker::ensure_started(&options.semantic.reranker_model_dir);
                                match reranker::runtime_snapshot(
                                    &options.semantic.reranker_model_dir,
                                ) {
                                    reranker::RuntimeSnapshot {
                                        phase: reranker::RuntimePhase::Ready,
                                        ..
                                    } => {
                                        let remaining_budget = semantic_deadline
                                            .saturating_duration_since(Instant::now());
                                        if remaining_budget.is_zero() {
                                            reranker_state = Some("timeout".to_string());
                                            append_fallback(
                                                &mut fallback_reason,
                                                "准确模式查询预算已用尽，本次返回均衡模式结果",
                                            );
                                        } else {
                                            let documents = candidates
                                                .iter()
                                                .map(|hit| {
                                                    let excerpt =
                                                        bounded_reranker_context(&hit.excerpt);
                                                    format!("{}\n{}", hit.relative_path, excerpt)
                                                })
                                                .collect::<Vec<_>>();
                                            #[cfg(test)]
                                            {
                                                reranker_input_diagnostics =
                                                    Some(summarize_reranker_inputs(&documents));
                                            }
                                            let rerank_started = Instant::now();
                                            match reranker::rerank(
                                                &options.semantic.reranker_model_dir,
                                                &options.query,
                                                documents,
                                                remaining_budget,
                                            )
                                            .await
                                            {
                                                Ok(ranking) => {
                                                    reranker_duration_ms =
                                                        Some(rerank_started.elapsed().as_millis()
                                                            as u64);
                                                    reranker_top_score =
                                                        ranking.first().map(|item| item.score);
                                                    hits = apply_reranker_ranking(
                                                        candidates,
                                                        ranking,
                                                        protected_exact,
                                                        options.max_results,
                                                    );
                                                    engine = "fts5+bge+reranker".to_string();
                                                    fusion = Some(ACCURATE_FUSION_NAME.to_string());
                                                    reranker_state = Some("ready".to_string());
                                                }
                                                Err(error) => {
                                                    reranker_duration_ms =
                                                        Some(rerank_started.elapsed().as_millis()
                                                            as u64);
                                                    reranker_state = Some(error.state);
                                                    append_fallback(
                                                        &mut fallback_reason,
                                                        &format!(
                                                            "准确模式重排失败，本次返回均衡模式结果: {}",
                                                            error.message
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    reranker::RuntimeSnapshot { phase, error } => {
                                        reranker_state = Some(phase.as_str().to_string());
                                        append_fallback(
                                            &mut fallback_reason,
                                            &format!(
                                                "准确模式模型状态为 {}，本次返回均衡模式结果{}",
                                                phase.as_str(),
                                                error
                                                    .map(|value| format!(": {}", value))
                                                    .unwrap_or_default()
                                            ),
                                        );
                                    }
                                }
                            }
                        } else {
                            hits = fuse_hits(
                                hits,
                                semantic_hits,
                                &options.query,
                                &terms,
                                options.max_results,
                            );
                        }
                    }
                    Err(error) => {
                        let resource_limited = error.starts_with("resource:");
                        let failure_state = if error.starts_with("loading:") {
                            SEMANTIC_BUILDING
                        } else if error.starts_with("missing:") {
                            SEMANTIC_MISSING
                        } else {
                            SEMANTIC_ERROR
                        };
                        index.semantic_state.store(failure_state, Ordering::Release);
                        if let Ok(mut last_error) = index.semantic_last_error.lock() {
                            *last_error = Some(error.clone());
                        }
                        if resource_limited {
                            let (fallback_hits, fallback_engine) =
                                run_immediate_search(&root, &options, &terms).await?;
                            hits = fallback_hits;
                            engine = fallback_engine;
                            degraded = true;
                            append_fallback(
                                &mut fallback_reason,
                                "资源压力较高，本次语义查询降级为 rg/Rust 全局项目匹配",
                            );
                        } else {
                            append_fallback(
                                &mut fallback_reason,
                                &format!("BGE 查询失败，本次保留 FTS5 结果: {}", error),
                            );
                        }
                    }
                }
            } else {
                let resource_limited = index
                    .semantic_last_error
                    .lock()
                    .ok()
                    .and_then(|error| error.clone())
                    .is_some_and(|error| error.starts_with("resource:"));
                if resource_limited {
                    let (fallback_hits, fallback_engine) =
                        run_immediate_search(&root, &options, &terms).await?;
                    hits = fallback_hits;
                    engine = fallback_engine;
                    degraded = true;
                    append_fallback(
                        &mut fallback_reason,
                        "资源压力较高，本次语义索引让路，使用 rg/Rust 全局项目匹配",
                    );
                } else {
                    append_fallback(
                        &mut fallback_reason,
                        &format!(
                            "语义索引状态为 {}，本次保留 FTS5 结果",
                            index.semantic_state_name(true)
                        ),
                    );
                }
            }
        }
    }
    if !matches!(engine.as_str(), "fts5+bge" | "fts5+bge+reranker") {
        hits.truncate(options.max_results.max(1));
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let state = index.state_name().to_string();
    let semantic_state = index
        .semantic_state_name(options.semantic.enabled())
        .to_string();
    let semantic_indexed_chunks = index.semantic_indexed_chunks.load(Ordering::Acquire);
    let semantic_pending_chunks = index.semantic_pending_chunks.load(Ordering::Acquire);
    let text = format_hits(
        &root,
        &hits,
        &engine,
        &state,
        duration_ms,
        fallback_reason.as_deref(),
        notice.as_deref(),
        &semantic_state,
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_top_score,
        semantic_mode.as_str(),
        reranker_state.as_deref(),
        reranker_model.as_deref(),
        reranker_duration_ms,
        reranker_top_score,
        fusion.as_deref(),
    );
    Ok(LocalSearchOutput {
        text,
        hit_count: hits.len(),
        engine,
        index_state: state,
        degraded,
        fallback_reason,
        notice,
        duration_ms,
        semantic_state,
        semantic_model: options.semantic.enabled().then(|| semantic::model_key()),
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_top_score,
        semantic_mode: semantic_mode.as_str().to_string(),
        reranker_state,
        reranker_model,
        reranker_duration_ms,
        reranker_top_score,
        #[cfg(test)]
        reranker_input_diagnostics,
        fusion,
    })
}

pub(super) async fn search_immediate(options: LocalSearchOptions) -> Result<LocalSearchOutput> {
    let started_at = Instant::now();
    let root = options
        .project_root
        .canonicalize()
        .with_context(|| format!("即时搜索项目路径无效: {}", options.project_root.display()))?;
    if !root.is_dir() {
        return Err(anyhow!("即时搜索项目路径不是目录: {}", root.display()));
    }
    let terms = extract_query_terms(&options.query);
    if terms.is_empty() {
        return Err(anyhow!("本地搜索未提取到有效关键词"));
    }
    let (hits, engine) = run_immediate_search(&root, &options, &terms).await?;
    let duration_ms = started_at.elapsed().as_millis() as u64;
    let notice = "工作区直属文件使用即时词法搜索，不建立重复父级索引".to_string();
    let text = format_hits(
        &root,
        &hits,
        &engine,
        "live",
        duration_ms,
        None,
        Some(&notice),
        "not_applicable",
        0,
        0,
        None,
        options.semantic.mode.as_str(),
        options.semantic.mode.accurate().then_some("skipped"),
        options
            .semantic
            .mode
            .accurate()
            .then_some(reranker::MODEL_NAME),
        None,
        None,
        None,
    );
    Ok(LocalSearchOutput {
        text,
        hit_count: hits.len(),
        engine,
        index_state: "live".to_string(),
        degraded: false,
        fallback_reason: None,
        notice: Some(notice),
        duration_ms,
        semantic_state: "not_applicable".to_string(),
        semantic_model: None,
        semantic_indexed_chunks: 0,
        semantic_pending_chunks: 0,
        semantic_top_score: None,
        semantic_mode: options.semantic.mode.as_str().to_string(),
        reranker_state: options
            .semantic
            .mode
            .accurate()
            .then(|| "skipped".to_string()),
        reranker_model: options
            .semantic
            .mode
            .accurate()
            .then(|| reranker::MODEL_NAME.to_string()),
        reranker_duration_ms: None,
        reranker_top_score: None,
        #[cfg(test)]
        reranker_input_diagnostics: None,
        fusion: None,
    })
}

pub async fn rebuild(
    project_root: &str,
    exclude_paths: Vec<String>,
    index_dir: PathBuf,
    semantic_settings: LocalSemanticSettings,
) -> Result<LocalIndexStatus> {
    let root = PathBuf::from(project_root)
        .canonicalize()
        .with_context(|| format!("本地索引项目路径无效: {}", project_root))?;
    let index = project_index(&root, &index_dir)?;
    refresh_profile(&index, &exclude_paths);
    sync_now(Arc::clone(&index), exclude_paths, semantic_settings.clone()).await?;
    Ok(index.status(&semantic_settings))
}

pub fn status(
    project_root: &str,
    index_dir: PathBuf,
    semantic_settings: LocalSemanticSettings,
) -> Result<LocalIndexStatus> {
    let root = PathBuf::from(project_root)
        .canonicalize()
        .with_context(|| format!("本地索引项目路径无效: {}", project_root))?;
    let cached = PROJECT_INDEXES
        .lock()
        .map_err(|_| anyhow!("本地索引管理器锁已损坏"))?
        .get(&root)
        .cloned();
    if let Some(index) = cached {
        return Ok(index.status(&semantic_settings));
    }
    let db_path = index_dir.join(format!("{}.sqlite3", project_hash(&root)));
    Ok(ProjectIndex::new(root, db_path).status(&semantic_settings))
}

pub fn workspace_status(
    project_root: &str,
    exclude_paths: Vec<String>,
    index_dir: PathBuf,
    semantic_settings: LocalSemanticSettings,
) -> Result<LocalIndexStatus> {
    let layout =
        crate::mcp::tools::workspace::resolve_workspace(Path::new(project_root), &exclude_paths)?;
    if !layout.is_workspace {
        return status(project_root, index_dir, semantic_settings);
    }

    let mut statuses = Vec::new();
    for project in &layout.projects {
        let status = status(
            &normalize_path(&project.root),
            index_dir.clone(),
            semantic_settings.clone(),
        )?;
        statuses.push((project, status));
    }
    aggregate_workspace_status(&layout, &index_dir, &semantic_settings, statuses)
}

pub async fn rebuild_workspace(
    project_root: &str,
    exclude_paths: Vec<String>,
    index_dir: PathBuf,
    semantic_settings: LocalSemanticSettings,
) -> Result<LocalIndexStatus> {
    let layout =
        crate::mcp::tools::workspace::resolve_workspace(Path::new(project_root), &exclude_paths)?;
    if !layout.is_workspace {
        return rebuild(project_root, exclude_paths, index_dir, semantic_settings).await;
    }
    archive_workspace_parent_index(&layout.root, &index_dir)?;

    for project in &layout.projects {
        rebuild(
            &normalize_path(&project.root),
            exclude_paths.clone(),
            index_dir.clone(),
            semantic_settings.clone(),
        )
        .await?;
    }
    workspace_status(
        &normalize_path(&layout.root),
        exclude_paths,
        index_dir,
        semantic_settings,
    )
}

fn aggregate_workspace_status(
    layout: &crate::mcp::tools::workspace::WorkspaceLayout,
    index_dir: &Path,
    semantic_settings: &LocalSemanticSettings,
    statuses: Vec<(
        &crate::mcp::tools::workspace::WorkspaceProject,
        LocalIndexStatus,
    )>,
) -> Result<LocalIndexStatus> {
    let embedding_snapshot = crate::mcp::embedding::snapshot(&semantic_settings.model_dir);
    let indexed_files = statuses
        .iter()
        .map(|(_, status)| status.indexed_files)
        .sum();
    let indexed_chunks = statuses
        .iter()
        .map(|(_, status)| status.indexed_chunks)
        .sum();
    let semantic_indexed_chunks = statuses
        .iter()
        .map(|(_, status)| status.semantic_indexed_chunks)
        .sum();
    let semantic_pending_chunks = statuses
        .iter()
        .map(|(_, status)| status.semantic_pending_chunks)
        .sum();
    let sync_running = statuses.iter().any(|(_, status)| status.sync_running);
    let lexical_sync_running = statuses
        .iter()
        .any(|(_, status)| status.lexical_sync_running);
    let semantic_sync_running = statuses
        .iter()
        .any(|(_, status)| status.semantic_sync_running);
    let pending_changes = statuses.iter().any(|(_, status)| status.pending_changes);
    let state = if statuses.iter().any(|(_, status)| status.state == "error") {
        "error"
    } else if statuses.iter().all(|(_, status)| status.state == "ready") {
        "ready"
    } else {
        "partial"
    };
    let semantic_states = statuses
        .iter()
        .map(|(_, status)| status.semantic_state.clone())
        .collect::<Vec<_>>();
    let semantic_state = aggregate_status_name(&semantic_states);
    let errors = statuses
        .iter()
        .filter_map(|(project, status)| {
            status
                .last_error
                .as_ref()
                .map(|error| format!("{}: {}", project.name(), error))
        })
        .collect::<Vec<_>>();
    let semantic_errors = statuses
        .iter()
        .filter_map(|(project, status)| {
            status
                .semantic_last_error
                .as_ref()
                .map(|error| format!("{}: {}", project.name(), error))
        })
        .collect::<Vec<_>>();
    let scopes = statuses
        .into_iter()
        .map(|(project, status)| LocalIndexScopeStatus {
            name: project.name(),
            relative_path: project.relative_path.clone(),
            project_root: status.project_root,
            index_path: status.index_path,
            state: status.state,
            indexed_files: status.indexed_files,
            indexed_chunks: status.indexed_chunks,
            lexical_sync_running: status.lexical_sync_running,
            pending_changes: status.pending_changes,
            semantic_state: status.semantic_state,
            semantic_indexed_chunks: status.semantic_indexed_chunks,
            semantic_pending_chunks: status.semantic_pending_chunks,
            last_error: status.last_error,
        })
        .collect();

    Ok(LocalIndexStatus {
        is_workspace: true,
        project_count: layout.projects.len(),
        scopes,
        project_root: normalize_path(&layout.root),
        index_path: normalize_path(index_dir),
        state: state.to_string(),
        indexed_files,
        indexed_chunks,
        sync_running,
        lexical_sync_running,
        semantic_sync_running,
        pending_changes,
        last_error: (!errors.is_empty()).then(|| errors.join("；")),
        semantic_state,
        semantic_model: semantic_settings.enabled().then(semantic::model_key),
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_last_error: (!semantic_errors.is_empty()).then(|| semantic_errors.join("；")),
        semantic_requested_provider: embedding_snapshot.requested_provider,
        semantic_execution_provider: embedding_snapshot.execution_provider,
        semantic_provider_fallback_reason: embedding_snapshot.provider_fallback_reason,
        semantic_cuda_runtime_available: embedding_snapshot.cuda_runtime_available,
        semantic_cuda_runtime_dir: embedding_snapshot
            .cuda_runtime_dir
            .as_deref()
            .map(normalize_path),
        semantic_cuda_runtime_error: embedding_snapshot.cuda_runtime_error,
        semantic_batch_size: embedding_snapshot.batch_size,
        semantic_intra_threads: embedding_snapshot.intra_threads,
    })
}

pub(crate) fn has_workspace_parent_index(project_root: &Path, index_dir: &Path) -> Result<bool> {
    let root = project_root
        .canonicalize()
        .with_context(|| format!("工作区路径无效: {}", project_root.display()))?;
    Ok(index_dir
        .join(format!("{}.sqlite3", project_hash(&root)))
        .exists())
}

pub(crate) fn archive_workspace_parent_index(
    project_root: &Path,
    index_dir: &Path,
) -> Result<Option<PathBuf>> {
    let root = project_root
        .canonicalize()
        .with_context(|| format!("工作区路径无效: {}", project_root.display()))?;
    let db_path = index_dir.join(format!("{}.sqlite3", project_hash(&root)));
    let wal_path = PathBuf::from(format!("{}-wal", db_path.to_string_lossy()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.to_string_lossy()));
    if ![&db_path, &wal_path, &shm_path]
        .iter()
        .any(|path| path.exists())
    {
        return Ok(None);
    }

    let mut indexes = PROJECT_INDEXES
        .lock()
        .map_err(|_| anyhow!("本地索引管理器锁已损坏"))?;
    if indexes
        .get(&root)
        .is_some_and(|index| index.sync_running.load(Ordering::Acquire))
    {
        return Err(anyhow!("父目录本地索引仍在同步，迁移已延后"));
    }
    indexes.remove(&root);
    drop(indexes);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let archive_dir = index_dir.join("archive").join(format!(
        "workspace-parent-{}-{}",
        timestamp,
        project_hash(&root)
    ));
    fs::create_dir_all(&archive_dir).context("创建父目录本地索引归档目录失败")?;
    for source in [&db_path, &wal_path, &shm_path] {
        if !source.exists() {
            continue;
        }
        let file_name = source
            .file_name()
            .ok_or_else(|| anyhow!("父目录本地索引文件名无效: {}", source.display()))?;
        fs::rename(source, archive_dir.join(file_name))
            .with_context(|| format!("归档父目录本地索引失败: {}", source.display()))?;
    }
    Ok(Some(archive_dir))
}

fn aggregate_status_name(states: &[String]) -> String {
    if states.is_empty() {
        return "not_applicable".to_string();
    }
    if states.iter().any(|state| state == "error") {
        "error"
    } else if states
        .iter()
        .any(|state| matches!(state.as_str(), "building" | "syncing"))
    {
        "syncing"
    } else if states.iter().all(|state| state == "ready") {
        "ready"
    } else if states.iter().all(|state| state == "disabled") {
        "disabled"
    } else if states.iter().any(|state| state == "missing") {
        "missing"
    } else {
        "partial"
    }
    .to_string()
}

fn project_index(root: &Path, index_dir: &Path) -> Result<Arc<ProjectIndex>> {
    let mut indexes = PROJECT_INDEXES
        .lock()
        .map_err(|_| anyhow!("本地索引管理器锁已损坏"))?;
    fs::create_dir_all(&index_dir).context("创建 sou 本地索引目录失败")?;
    let db_path = index_dir.join(format!("{}.sqlite3", project_hash(root)));
    if let Some(index) = indexes.get(root) {
        if index.db_path == db_path {
            return Ok(Arc::clone(index));
        }
    }

    // 中文说明：目录切换时替换当前项目实例，旧数据库文件保持原样，便于用户随时切回。
    let index = Arc::new(ProjectIndex::new(root.to_path_buf(), db_path));
    indexes.insert(root.to_path_buf(), Arc::clone(&index));
    Ok(index)
}

fn refresh_profile(index: &ProjectIndex, excludes: &[String]) {
    let profile = profile_hash(excludes);
    if let Ok(mut current) = index.profile_hash.lock() {
        if current.is_empty() {
            *current = profile;
        } else if *current != profile {
            *current = profile;
            index.dirty.store(true, Ordering::Release);
        }
    }
}

enum SemanticSyncOutcome {
    Disabled,
    Ready(semantic::SemanticSyncStats),
    Missing(String),
    Error(String),
}

struct SyncOutcome {
    files: u64,
    chunks: u64,
    semantic: SemanticSyncOutcome,
}

fn schedule_sync(
    index: Arc<ProjectIndex>,
    exclude_paths: Vec<String>,
    semantic_settings: LocalSemanticSettings,
) {
    if index
        .sync_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    // 成功占有同步任务后再清除 dirty；同步期间的新事件会重新置位，不会丢失后续变更。
    index.dirty.store(false, Ordering::Release);
    index.lexical_sync_running.store(true, Ordering::Release);
    if index.state.load(Ordering::Acquire) != INDEX_READY {
        index.state.store(INDEX_BUILDING, Ordering::Release);
    }
    if semantic_settings.enabled() {
        let next_state = if index.semantic_state.load(Ordering::Acquire) == SEMANTIC_READY {
            SEMANTIC_SYNCING
        } else {
            SEMANTIC_BUILDING
        };
        index.semantic_state.store(next_state, Ordering::Release);
    }

    tokio::spawn(async move {
        let task_index = Arc::clone(&index);
        let result = tokio::task::spawn_blocking(move || {
            sync_all(&task_index, &exclude_paths, &semantic_settings)
        })
        .await
        .map_err(|error| anyhow!("本地索引同步任务异常: {}", error))
        .and_then(|value| value);
        finish_sync(&index, result);
    });
}

async fn sync_now(
    index: Arc<ProjectIndex>,
    exclude_paths: Vec<String>,
    semantic_settings: LocalSemanticSettings,
) -> Result<()> {
    if index
        .sync_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(anyhow!("本地索引正在同步，请稍后重试"));
    }
    // 当前同步任务消费既有 dirty；同步期间的新文件事件仍会再次置位。
    index.dirty.store(false, Ordering::Release);
    index.lexical_sync_running.store(true, Ordering::Release);
    index.state.store(INDEX_BUILDING, Ordering::Release);
    if semantic_settings.enabled() {
        let next_state = if index.semantic_state.load(Ordering::Acquire) == SEMANTIC_READY {
            SEMANTIC_SYNCING
        } else {
            SEMANTIC_BUILDING
        };
        index.semantic_state.store(next_state, Ordering::Release);
    }
    let task_index = Arc::clone(&index);
    let result = tokio::task::spawn_blocking(move || {
        sync_all(&task_index, &exclude_paths, &semantic_settings)
    })
    .await
    .map_err(|error| anyhow!("本地索引同步任务异常: {}", error))
    .and_then(|value| value);
    match result {
        Ok(counts) => {
            finish_sync(&index, Ok(counts));
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            finish_sync(&index, Err(error));
            Err(anyhow!(message))
        }
    }
}

fn sync_all(
    index: &ProjectIndex,
    exclude_paths: &[String],
    semantic_settings: &LocalSemanticSettings,
) -> Result<SyncOutcome> {
    let (files, chunks) = match sync_index(index, exclude_paths) {
        Ok(counts) => counts,
        Err(error) => {
            index.lexical_sync_running.store(false, Ordering::Release);
            return Err(error);
        }
    };
    index.indexed_files.store(files, Ordering::Release);
    index.indexed_chunks.store(chunks, Ordering::Release);
    index.state.store(INDEX_READY, Ordering::Release);
    index
        .startup_reconcile_pending
        .store(false, Ordering::Release);
    if let Ok(mut error) = index.last_error.lock() {
        *error = None;
    }
    // 词法快照已就绪后即可恢复 FTS5；向量写入仍由同一串行管线继续执行。
    index.lexical_sync_running.store(false, Ordering::Release);
    let semantic = if !semantic_settings.enabled() {
        index
            .semantic_state
            .store(SEMANTIC_DISABLED, Ordering::Release);
        SemanticSyncOutcome::Disabled
    } else if !crate::mcp::embedding::assets_available_for_provider(
        &semantic_settings.model_dir,
        crate::mcp::embedding::configured_provider(),
    ) {
        SemanticSyncOutcome::Missing("BGE 模型资产未就绪".to_string())
    } else {
        index
            .semantic_pending_chunks
            .store(chunks, Ordering::Release);
        match semantic::sync_vectors(
            &index.db_path,
            &semantic_settings.model_dir,
            |indexed, pending| {
                index
                    .semantic_indexed_chunks
                    .store(indexed, Ordering::Release);
                index
                    .semantic_pending_chunks
                    .store(pending, Ordering::Release);
            },
        ) {
            Ok(stats) => SemanticSyncOutcome::Ready(stats),
            Err(error) => {
                let message = error.to_string();
                if message.starts_with("missing:") {
                    SemanticSyncOutcome::Missing(message)
                } else {
                    SemanticSyncOutcome::Error(message)
                }
            }
        }
    };
    Ok(SyncOutcome {
        files,
        chunks,
        semantic,
    })
}

fn finish_sync(index: &ProjectIndex, result: Result<SyncOutcome>) {
    match result {
        Ok(outcome) => {
            index.indexed_files.store(outcome.files, Ordering::Release);
            index
                .indexed_chunks
                .store(outcome.chunks, Ordering::Release);
            index.state.store(INDEX_READY, Ordering::Release);
            if let Ok(mut error) = index.last_error.lock() {
                *error = None;
            }
            match outcome.semantic {
                SemanticSyncOutcome::Disabled => {
                    index
                        .semantic_state
                        .store(SEMANTIC_DISABLED, Ordering::Release);
                }
                SemanticSyncOutcome::Ready(stats) => {
                    index
                        .semantic_indexed_chunks
                        .store(stats.indexed_chunks, Ordering::Release);
                    index
                        .semantic_pending_chunks
                        .store(stats.pending_chunks, Ordering::Release);
                    index
                        .semantic_state
                        .store(SEMANTIC_READY, Ordering::Release);
                    if let Ok(mut error) = index.semantic_last_error.lock() {
                        *error = None;
                    }
                }
                SemanticSyncOutcome::Missing(message) => {
                    index
                        .semantic_state
                        .store(SEMANTIC_MISSING, Ordering::Release);
                    if let Ok(mut error) = index.semantic_last_error.lock() {
                        *error = Some(message);
                    }
                }
                SemanticSyncOutcome::Error(message) => {
                    index
                        .semantic_state
                        .store(SEMANTIC_ERROR, Ordering::Release);
                    if let Ok(mut error) = index.semantic_last_error.lock() {
                        *error = Some(message);
                    }
                }
            }
            log::info!(
                "[sou-local] 索引同步完成: files={}, chunks={}, semantic_state={}",
                outcome.files,
                outcome.chunks,
                index.semantic_state_name(true)
            );
        }
        Err(error) => {
            mark_index_error(index, &error.to_string());
            log::warn!("[sou-local] 索引同步失败: {}", error);
        }
    }
    index.lexical_sync_running.store(false, Ordering::Release);
    index.sync_running.store(false, Ordering::Release);
}

fn mark_index_error(index: &ProjectIndex, message: &str) {
    index.state.store(INDEX_ERROR, Ordering::Release);
    if let Ok(mut error) = index.last_error.lock() {
        *error = Some(message.to_string());
    }
}

fn inspect_existing_index(db_path: &Path) -> (u8, u64, u64, Option<String>) {
    if !db_path.is_file() {
        return (INDEX_MISSING, 0, 0, None);
    }
    let inspected = (|| -> Result<(u64, u64)> {
        // 状态轮询只读既有数据库，schema 初始化由搜索和显式重建入口负责。
        let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("只读打开本地索引失败: {}", db_path.display()))?;
        connection.busy_timeout(Duration::from_millis(250))?;
        index_counts(&connection)
    })();
    match inspected {
        Ok((files, chunks)) => (INDEX_READY, files, chunks, None),
        Err(error) => (INDEX_ERROR, 0, 0, Some(error.to_string())),
    }
}

fn open_database(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("打开本地索引失败: {}", path.display()))?;
    connection.busy_timeout(Duration::from_millis(250))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS files (
             path TEXT PRIMARY KEY,
             modified_ns INTEGER NOT NULL,
             size INTEGER NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(
             path UNINDEXED,
             start_line UNINDEXED,
             end_line UNINDEXED,
             search_text,
             content UNINDEXED,
             tokenize='unicode61 remove_diacritics 2'
         );",
    )?;
    semantic::ensure_schema(&connection)?;
    Ok(connection)
}

fn sync_index(index: &ProjectIndex, exclude_paths: &[String]) -> Result<(u64, u64)> {
    let mut connection = open_database(&index.db_path)?;
    let mut existing = load_file_metadata(&connection)?;
    let files = collect_project_files(&index.root, exclude_paths);
    let transaction = connection.transaction()?;

    for path in files {
        let relative = relative_path(&index.root, &path)?;
        let metadata = match fs::metadata(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let signature = (modified_ns(&metadata), metadata.len() as i64);
        if existing.remove(&relative) == Some(signature) {
            continue;
        }

        transaction.execute(
            "DELETE FROM chunk_vectors WHERE path = ?1",
            params![relative],
        )?;
        transaction.execute("DELETE FROM chunks WHERE path = ?1", params![relative])?;
        transaction.execute("DELETE FROM files WHERE path = ?1", params![relative])?;
        let Some(content) = read_text_file(&path, metadata.len())? else {
            continue;
        };
        transaction.execute(
            "INSERT INTO files(path, modified_ns, size) VALUES (?1, ?2, ?3)",
            params![relative, signature.0, signature.1],
        )?;
        for (start_line, end_line, excerpt) in chunk_content(&content) {
            let search_text = build_search_text(&relative, &excerpt);
            transaction.execute(
                "INSERT INTO chunks(path, start_line, end_line, search_text, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    relative,
                    start_line as i64,
                    end_line as i64,
                    search_text,
                    excerpt
                ],
            )?;
        }
    }

    for stale in existing.keys() {
        transaction.execute("DELETE FROM chunk_vectors WHERE path = ?1", params![stale])?;
        transaction.execute("DELETE FROM chunks WHERE path = ?1", params![stale])?;
        transaction.execute("DELETE FROM files WHERE path = ?1", params![stale])?;
    }
    transaction.commit()?;
    index_counts(&connection)
}

fn load_file_metadata(connection: &Connection) -> Result<HashMap<String, (i64, i64)>> {
    let mut statement = connection.prepare("SELECT path, modified_ns, size FROM files")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
        ))
    })?;
    let mut values = HashMap::new();
    for row in rows {
        let (path, signature) = row?;
        values.insert(path, signature);
    }
    Ok(values)
}

fn index_counts(connection: &Connection) -> Result<(u64, u64)> {
    let files =
        connection.query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, u64>(0))?;
    let chunks = connection.query_row("SELECT COUNT(*) FROM chunks", [], |row| {
        row.get::<_, u64>(0)
    })?;
    Ok((files, chunks))
}

fn query_index(
    db_path: &Path,
    query: &str,
    terms: &[String],
    max_results: usize,
) -> Result<Vec<SearchHit>> {
    let connection = open_database(db_path)?;
    let match_query = terms
        .iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let fetch_limit = max_results.max(1).saturating_mul(5).min(150);
    let mut statement = connection.prepare(
        "SELECT path, start_line, end_line, content,
                bm25(chunks, 0.0, 0.0, 0.0, 1.0, 0.0) AS lexical_score
         FROM chunks
         WHERE chunks MATCH ?1
         ORDER BY lexical_score
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![match_query, fetch_limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as usize,
            row.get::<_, i64>(2)? as usize,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
        ))
    })?;

    let mut hits = Vec::new();
    for row in rows {
        let (path, start_line, end_line, excerpt, lexical_score) = row?;
        hits.push(score_hit(
            path,
            start_line,
            end_line,
            excerpt,
            lexical_score,
            query,
            terms,
        ));
    }
    rank_and_limit(hits, max_results)
}

async fn run_immediate_search(
    root: &Path,
    options: &LocalSearchOptions,
    terms: &[String],
) -> Result<(Vec<SearchHit>, String)> {
    match run_rg(
        root,
        &options.query,
        terms,
        options.max_results,
        &options.exclude_paths,
    )
    .await
    {
        Ok(hits) => Ok((hits, "rg".to_string())),
        Err(error) => {
            log::warn!("[sou-local] rg 即时搜索失败，切换 Rust 扫描: {}", error);
            let root = root.to_path_buf();
            let query = options.query.clone();
            let terms = terms.to_vec();
            let excludes = options.exclude_paths.clone();
            let max_results = options.max_results;
            let hits = tokio::task::spawn_blocking(move || {
                scan_project(&root, &query, &terms, max_results, &excludes)
            })
            .await
            .context("等待 Rust 本地扫描任务失败")??;
            Ok((hits, "scan".to_string()))
        }
    }
}

async fn run_rg(
    root: &Path,
    query: &str,
    terms: &[String],
    max_results: usize,
    excludes: &[String],
) -> Result<Vec<SearchHit>> {
    let mut command = Command::new("rg");
    command
        .current_dir(root)
        .kill_on_drop(true)
        .arg("--json")
        .arg("--line-number")
        .arg("--ignore-case")
        .arg("--fixed-strings")
        .arg("--no-messages")
        .arg("--max-count")
        .arg("4")
        .arg("--max-filesize")
        .arg("1M")
        .arg("--glob")
        .arg(code_glob());
    for term in terms.iter().take(12) {
        command.arg("-e").arg(term);
    }
    for exclude in excludes {
        command.arg("--glob").arg(exclude_glob(exclude));
    }
    command
        .arg(".")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let output = tokio::time::timeout(Duration::from_secs(3), command.output())
        .await
        .map_err(|_| anyhow!("rg 即时搜索超时"))?
        .context("启动 rg 失败")?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(anyhow!("rg 退出码异常: {:?}", output.status.code()));
    }

    let mut matches: HashMap<String, Vec<usize>> = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let Some(data) = event.get("data") else {
            continue;
        };
        let Some(path) = data
            .get("path")
            .and_then(|value| value.get("text"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(line_number) = data.get("line_number").and_then(Value::as_u64) else {
            continue;
        };
        let path = normalize_relative(path);
        let entry = matches.entry(path).or_default();
        if entry.len() < 4 {
            entry.push(line_number as usize);
        }
    }

    hits_from_line_matches(root, query, terms, matches, max_results)
}

fn scan_project(
    root: &Path,
    query: &str,
    terms: &[String],
    max_results: usize,
    excludes: &[String],
) -> Result<Vec<SearchHit>> {
    let mut matches = HashMap::new();
    for path in collect_project_files(root, excludes) {
        let metadata = match fs::metadata(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(content) = read_text_file(&path, metadata.len())? else {
            continue;
        };
        let mut lines = Vec::new();
        for (index, line) in content.lines().enumerate() {
            let lower = line.to_lowercase();
            if terms.iter().any(|term| lower.contains(term)) {
                lines.push(index + 1);
                if lines.len() == 4 {
                    break;
                }
            }
        }
        if !lines.is_empty() {
            matches.insert(relative_path(root, &path)?, lines);
        }
    }
    hits_from_line_matches(root, query, terms, matches, max_results)
}

fn hits_from_line_matches(
    root: &Path,
    query: &str,
    terms: &[String],
    matches: HashMap<String, Vec<usize>>,
    max_results: usize,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    for (path, mut line_numbers) in matches {
        line_numbers.sort_unstable();
        line_numbers.dedup();
        let full_path = root.join(&path);
        let content = fs::read_to_string(&full_path)
            .with_context(|| format!("读取即时搜索命中文件失败: {}", full_path.display()))?;
        let all_lines = content.lines().collect::<Vec<_>>();
        for line_number in line_numbers.into_iter().take(2) {
            let start_line = line_number.saturating_sub(3).max(1);
            let end_line = (line_number + 3).min(all_lines.len());
            let excerpt = all_lines[start_line - 1..end_line].join("\n");
            hits.push(score_hit(
                path.clone(),
                start_line,
                end_line,
                excerpt,
                0.0,
                query,
                terms,
            ));
        }
    }
    rank_and_limit(hits, max_results)
}

fn score_hit(
    relative_path: String,
    start_line: usize,
    end_line: usize,
    excerpt: String,
    lexical_score: f64,
    query: &str,
    terms: &[String],
) -> SearchHit {
    let lower_excerpt = excerpt.to_lowercase();
    let lower_path = relative_path.to_lowercase();
    let coverage = terms
        .iter()
        .filter(|term| lower_excerpt.contains(term.as_str()) || lower_path.contains(term.as_str()))
        .count();
    let path_matches = terms
        .iter()
        .filter(|term| lower_path.contains(term.as_str()))
        .count();
    let normalized_query = query.trim().to_lowercase();
    let exact_match = !normalized_query.is_empty()
        && (lower_excerpt.contains(&normalized_query) || lower_path.contains(&normalized_query));
    SearchHit {
        relative_path,
        start_line,
        end_line,
        excerpt,
        coverage,
        exact_match,
        path_matches,
        lexical_score,
        semantic_score: None,
        fusion_score: 0.0,
    }
}

fn rank_and_limit(mut hits: Vec<SearchHit>, max_results: usize) -> Result<Vec<SearchHit>> {
    hits.sort_by(|left, right| {
        right
            .coverage
            .cmp(&left.coverage)
            .then_with(|| right.exact_match.cmp(&left.exact_match))
            .then_with(|| right.path_matches.cmp(&left.path_matches))
            .then_with(|| {
                left.lexical_score
                    .partial_cmp(&right.lexical_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
    let mut seen = HashSet::new();
    hits.retain(|hit| seen.insert((hit.relative_path.clone(), hit.start_line, hit.end_line)));
    hits.truncate(max_results.max(1));
    Ok(hits)
}

fn fuse_hits(
    lexical_hits: Vec<SearchHit>,
    semantic_hits: Vec<semantic::SemanticHit>,
    query: &str,
    terms: &[String],
    max_results: usize,
) -> Vec<SearchHit> {
    const RRF_K: f64 = 60.0;
    const LEXICAL_WEIGHT: f64 = 0.65;
    const SEMANTIC_WEIGHT: f64 = 0.35;

    let semantic_only = lexical_hits.is_empty();
    let mut merged: HashMap<(String, usize, usize), SearchHit> = HashMap::new();
    for (rank, mut hit) in lexical_hits.into_iter().enumerate() {
        hit.fusion_score = LEXICAL_WEIGHT / (RRF_K + rank as f64 + 1.0);
        let key = (hit.relative_path.clone(), hit.start_line, hit.end_line);
        merged.insert(key, hit);
    }
    for (rank, semantic_hit) in semantic_hits.into_iter().enumerate() {
        if semantic_only && semantic_hit.score < semantic::SEMANTIC_ONLY_THRESHOLD {
            continue;
        }
        let key = (
            semantic_hit.relative_path.clone(),
            semantic_hit.start_line,
            semantic_hit.end_line,
        );
        let contribution = SEMANTIC_WEIGHT / (RRF_K + rank as f64 + 1.0);
        let hit = merged.entry(key).or_insert_with(|| {
            score_hit(
                semantic_hit.relative_path.clone(),
                semantic_hit.start_line,
                semantic_hit.end_line,
                semantic_hit.excerpt.clone(),
                0.0,
                query,
                terms,
            )
        });
        hit.semantic_score = Some(semantic_hit.score);
        hit.fusion_score += contribution;
    }

    let mut hits = merged.into_values().collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .fusion_score
            .total_cmp(&left.fusion_score)
            .then_with(|| right.exact_match.cmp(&left.exact_match))
            .then_with(|| right.coverage.cmp(&left.coverage))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });
    hits.truncate(max_results.max(1));
    hits
}

fn select_accurate_rerank_candidates(retrieval_candidates: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen_paths = HashSet::new();
    retrieval_candidates
        .into_iter()
        .filter(|hit| seen_paths.insert(hit.relative_path.replace('\\', "/")))
        .take(ACCURATE_RERANK_LIMIT)
        .collect()
}

fn bounded_reranker_context(excerpt: &str) -> String {
    let chars = excerpt.chars().collect::<Vec<_>>();
    if chars.len() <= RERANKER_CONTEXT_CHARS {
        return excerpt.to_string();
    }

    // 中文说明：超长 JSON/SQL 只保留头尾，避免单个 chunk 让 tokenizer 扫描数十万字符。
    let head_len = RERANKER_CONTEXT_CHARS * 2 / 3;
    let tail_len = RERANKER_CONTEXT_CHARS - head_len;
    let head = chars[..head_len].iter().collect::<String>();
    let tail = chars[chars.len() - tail_len..].iter().collect::<String>();
    format!("{}\n[context-truncated]\n{}", head, tail)
}

#[cfg(test)]
fn summarize_reranker_inputs(documents: &[String]) -> RerankerInputDiagnostics {
    let mut char_counts = documents
        .iter()
        .map(|document| document.chars().count())
        .collect::<Vec<_>>();
    char_counts.sort_unstable();
    let percentile = |percent: usize| {
        char_counts
            .get(char_counts.len().saturating_sub(1) * percent / 100)
            .copied()
            .unwrap_or_default()
    };

    RerankerInputDiagnostics {
        candidate_count: documents.len(),
        batch_count: documents.len().div_ceil(reranker::BATCH_SIZE),
        truncated_candidate_count: documents
            .iter()
            .filter(|document| document.contains("[context-truncated]"))
            .count(),
        total_chars: char_counts.iter().sum(),
        min_chars: char_counts.first().copied().unwrap_or_default(),
        p50_chars: percentile(50),
        p95_chars: percentile(95),
        max_chars: char_counts.last().copied().unwrap_or_default(),
    }
}

fn accurate_query_budget() -> Duration {
    #[cfg(test)]
    if let Ok(value) = std::env::var("SANSHU_SOU_GATE_ACCURATE_OBSERVATION_MS") {
        if let Ok(milliseconds) = value.parse::<u64>() {
            // 中文说明：测试门禁可延长观测窗口，但正式查询始终使用固定 3 秒预算。
            if (3_000..=30_000).contains(&milliseconds) {
                return Duration::from_millis(milliseconds);
            }
        }
    }
    ACCURATE_QUERY_BUDGET
}

fn apply_reranker_ranking(
    mut candidates: Vec<SearchHit>,
    ranking: Vec<reranker::RerankMatch>,
    protected_exact: Option<SearchHit>,
    max_results: usize,
) -> Vec<SearchHit> {
    const RRF_K: f64 = 60.0;
    let mut reranker_ranks = HashMap::new();
    for (rank, item) in ranking.into_iter().enumerate() {
        if item.index < candidates.len() {
            reranker_ranks.entry(item.index).or_insert(rank);
        }
    }

    for (retrieval_rank, candidate) in candidates.iter_mut().enumerate() {
        let retrieval_score = RETRIEVAL_RRF_WEIGHT / (RRF_K + retrieval_rank as f64 + 1.0);
        let reranker_score = reranker_ranks
            .get(&retrieval_rank)
            .map(|rank| RERANKER_RRF_WEIGHT / (RRF_K + *rank as f64 + 1.0))
            .unwrap_or_default();
        candidate.fusion_score = retrieval_score + reranker_score;
    }
    candidates.sort_by(|left, right| {
        right
            .fusion_score
            .total_cmp(&left.fusion_score)
            .then_with(|| right.exact_match.cmp(&left.exact_match))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
            .then_with(|| left.start_line.cmp(&right.start_line))
    });

    if let Some(protected) = protected_exact {
        let protected_path = protected.relative_path.replace('\\', "/");
        candidates.retain(|hit| hit.relative_path.replace('\\', "/") != protected_path);
        candidates.insert(0, protected);
    }
    candidates.truncate(max_results.max(1));
    candidates
}

fn append_fallback(target: &mut Option<String>, reason: &str) {
    match target {
        Some(current) => {
            current.push_str("；");
            current.push_str(reason);
        }
        None => *target = Some(reason.to_string()),
    }
}

fn format_hits(
    root: &Path,
    hits: &[SearchHit],
    engine: &str,
    state: &str,
    duration_ms: u64,
    fallback_reason: Option<&str>,
    notice: Option<&str>,
    semantic_state: &str,
    semantic_indexed_chunks: u64,
    semantic_pending_chunks: u64,
    semantic_top_score: Option<f32>,
    semantic_mode: &str,
    reranker_state: Option<&str>,
    reranker_model: Option<&str>,
    reranker_duration_ms: Option<u64>,
    reranker_top_score: Option<f32>,
    fusion: Option<&str>,
) -> String {
    let mut parts = vec![
        "The following code sections were retrieved:".to_string(),
        String::new(),
    ];
    for hit in hits {
        parts.push(format!(
            "Path: {}",
            normalize_path(&root.join(&hit.relative_path))
        ));
        parts.push(format!("Lines: L{}-L{}", hit.start_line, hit.end_line));
        for (offset, line) in hit.excerpt.lines().enumerate() {
            parts.push(format!("L{}:{}", hit.start_line + offset, line));
        }
        parts.push(String::new());
    }
    if hits.is_empty() {
        parts.push("No relevant files found.".to_string());
    }
    parts.push(format!(
        "[sou-local] engine={}, index_state={}, hits={}, duration_ms={}, semantic_mode={}, semantic_state={}, semantic_indexed_chunks={}, semantic_pending_chunks={}{}{}{}{}{}{}",
        engine,
        state,
        hits.len(),
        duration_ms,
        semantic_mode,
        semantic_state,
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_top_score
            .map(|score| format!(", semantic_top_score={:.4}", score))
            .unwrap_or_default(),
        reranker_state
            .map(|value| format!(", reranker_state={}", value))
            .unwrap_or_default(),
        reranker_model
            .map(|value| format!(", reranker_model={}", value))
            .unwrap_or_default(),
        reranker_duration_ms
            .map(|value| format!(", reranker_duration_ms={}", value))
            .unwrap_or_default(),
        reranker_top_score
            .map(|score| format!(", reranker_top_score={:.4}", score))
            .unwrap_or_default(),
        fusion
            .map(|value| format!(", fusion={}", value))
            .unwrap_or_default()
    ));
    if let Some(reason) = fallback_reason {
        parts.push(format!("[sou-local fallback] {}", reason));
    }
    if let Some(message) = notice {
        parts.push(format!("[sou-local notice] {}", message));
    }
    parts.join("\n")
}

fn collect_project_files(root: &Path, excludes: &[String]) -> Vec<PathBuf> {
    WalkBuilder::new(root)
        .standard_filters(true)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path())
        .filter(|path| is_supported_file(path))
        .filter(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len() <= MAX_FILE_BYTES)
                .unwrap_or(false)
        })
        .filter(|path| !is_excluded(root, path, excludes))
        .collect()
}

fn is_supported_file(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    if extension.as_deref().is_some_and(|value| {
        matches!(
            value,
            "rs" | "c"
                | "cc"
                | "cpp"
                | "cxx"
                | "h"
                | "hh"
                | "hpp"
                | "cs"
                | "go"
                | "java"
                | "kt"
                | "kts"
                | "swift"
                | "scala"
                | "py"
                | "rb"
                | "php"
                | "lua"
                | "js"
                | "mjs"
                | "cjs"
                | "ts"
                | "tsx"
                | "jsx"
                | "vue"
                | "svelte"
                | "astro"
                | "html"
                | "css"
                | "scss"
                | "sass"
                | "less"
                | "sql"
                | "graphql"
                | "gql"
                | "proto"
                | "xml"
                | "json"
                | "jsonc"
                | "yaml"
                | "yml"
                | "toml"
                | "ini"
                | "md"
                | "mdx"
                | "txt"
                | "rst"
                | "adoc"
                | "sh"
                | "bash"
                | "zsh"
                | "fish"
                | "ps1"
                | "bat"
        )
    }) {
        return true;
    }
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some("Dockerfile" | "Makefile" | "CMakeLists.txt" | "Justfile")
    )
}

fn is_excluded(root: &Path, path: &Path, excludes: &[String]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let normalized = normalize_path(relative);
    excludes.iter().any(|exclude| {
        let exclude = exclude
            .trim()
            .trim_start_matches("./")
            .trim_matches('/')
            .replace('\\', "/");
        if exclude.is_empty() {
            return false;
        }
        let plain = exclude.trim_matches('*').trim_matches('/');
        normalized == plain
            || normalized.starts_with(&format!("{}/", plain))
            || normalized.contains(&format!("/{}/", plain))
            || normalized.ends_with(&format!("/{}", plain))
    })
}

fn read_text_file(path: &Path, size: u64) -> Result<Option<String>> {
    if size > MAX_FILE_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("读取源码失败: {}", path.display()))?;
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn chunk_content(content: &str) -> Vec<(usize, usize, String)> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let step = CHUNK_LINES - CHUNK_OVERLAP;
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        chunks.push((start + 1, end, lines[start..end].join("\n")));
        if end == lines.len() {
            break;
        }
        start += step;
    }
    chunks
}

fn build_search_text(path: &str, content: &str) -> String {
    tokenize_text(&format!("{}\n{}", path, content), usize::MAX).join(" ")
}

pub(super) fn extract_query_terms(query: &str) -> Vec<String> {
    let stopwords = [
        "the", "and", "for", "from", "with", "this", "that", "what", "where", "when", "代码",
        "项目", "搜索", "相关", "实现", "如何", "怎么", "什么", "是否",
    ];
    let mut terms = tokenize_text(query, usize::MAX)
        .into_iter()
        .filter(|term| term.len() >= 2 && !stopwords.contains(&term.as_str()))
        .collect::<Vec<_>>();
    // 混合长句优先保留代码标识符，其次保留中文二/三元词，避免自然语言前缀挤掉后置函数名。
    terms.sort_by_key(|term| {
        if term.is_ascii() {
            0
        } else if (2..=3).contains(&term.chars().count()) {
            1
        } else {
            2
        }
    });
    terms.truncate(MAX_QUERY_TERMS);
    terms
}

fn tokenize_text(text: &str, limit: usize) -> Vec<String> {
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() && output.len() < limit {
        if chars[index].is_ascii_alphanumeric()
            || matches!(chars[index], '_' | '-' | '.' | '/' | ':' | '\\')
        {
            let start = index;
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric()
                    || matches!(chars[index], '_' | '-' | '.' | '/' | ':' | '\\'))
            {
                index += 1;
            }
            let raw = chars[start..index].iter().collect::<String>();
            push_ascii_tokens(&raw, &mut output, &mut seen, limit);
            continue;
        }
        if is_cjk(chars[index]) {
            let start = index;
            index += 1;
            while index < chars.len() && is_cjk(chars[index]) {
                index += 1;
            }
            let run = chars[start..index].iter().collect::<String>();
            push_token(&run, &mut output, &mut seen, limit);
            let run_chars = run.chars().collect::<Vec<_>>();
            for pair in run_chars.windows(2) {
                push_token(
                    &pair.iter().collect::<String>(),
                    &mut output,
                    &mut seen,
                    limit,
                );
            }
            for triple in run_chars.windows(3) {
                push_token(
                    &triple.iter().collect::<String>(),
                    &mut output,
                    &mut seen,
                    limit,
                );
            }
            continue;
        }
        index += 1;
    }
    output
}

fn push_ascii_tokens(
    raw: &str,
    output: &mut Vec<String>,
    seen: &mut HashSet<String>,
    limit: usize,
) {
    let trimmed = raw.trim_matches(|ch: char| matches!(ch, '.' | '/' | ':' | '\\' | '-' | '_'));
    if trimmed.is_empty() {
        return;
    }
    push_token(&trimmed.to_ascii_lowercase(), output, seen, limit);
    for segment in trimmed.split(['_', '-', '.', '/', ':', '\\']) {
        if segment.is_empty() {
            continue;
        }
        push_token(&segment.to_ascii_lowercase(), output, seen, limit);
        for part in split_camel_case(segment) {
            push_token(&part.to_ascii_lowercase(), output, seen, limit);
        }
    }
}

fn split_camel_case(value: &str) -> Vec<String> {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() < 2 {
        return vec![value.to_string()];
    }
    let mut parts = Vec::new();
    let mut start = 0usize;
    for index in 1..chars.len() {
        let previous = chars[index - 1];
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        let boundary = (previous.is_ascii_lowercase() || previous.is_ascii_digit())
            && current.is_ascii_uppercase()
            || previous.is_ascii_uppercase()
                && current.is_ascii_uppercase()
                && next.is_some_and(|value| value.is_ascii_lowercase());
        if boundary {
            parts.push(chars[start..index].iter().collect());
            start = index;
        }
    }
    parts.push(chars[start..].iter().collect());
    parts
}

fn push_token(value: &str, output: &mut Vec<String>, seen: &mut HashSet<String>, limit: usize) {
    let value = value.trim().to_lowercase();
    if value.is_empty() || output.len() >= limit || !seen.insert(value.clone()) {
        return;
    }
    output.push(value);
}

fn is_cjk(value: char) -> bool {
    matches!(value as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff)
}

fn modified_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    Ok(normalize_path(path.strip_prefix(root).with_context(
        || format!("文件不在项目目录内: {}", path.display()),
    )?))
}

fn normalize_relative(path: &str) -> String {
    path.trim_start_matches("./").replace('\\', "/")
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn project_hash(root: &Path) -> String {
    let mut normalized = normalize_path(root);
    if cfg!(windows) {
        normalized = normalized.to_ascii_lowercase();
    }
    let mut context = ShaContext::new(&SHA256);
    context.update(normalized.as_bytes());
    hex::encode(&context.finish().as_ref()[..16])
}

fn profile_hash(excludes: &[String]) -> String {
    let mut values = excludes
        .iter()
        .map(|value| value.trim().replace('\\', "/"))
        .collect::<Vec<_>>();
    values.sort();
    let mut context = ShaContext::new(&SHA256);
    context.update(values.join("\n").as_bytes());
    hex::encode(&context.finish().as_ref()[..8])
}

fn code_glob() -> &'static str {
    "*.{rs,c,cc,cpp,cxx,h,hh,hpp,cs,go,java,kt,kts,swift,scala,py,rb,php,lua,js,mjs,cjs,ts,tsx,jsx,vue,svelte,astro,html,css,scss,sass,less,sql,graphql,gql,proto,xml,json,jsonc,yaml,yml,toml,ini,md,mdx,txt,rst,adoc,sh,bash,zsh,fish,ps1,bat}"
}

fn exclude_glob(value: &str) -> String {
    let normalized = value
        .trim()
        .trim_start_matches("./")
        .trim_matches('/')
        .replace('\\', "/");
    if normalized.starts_with('!') {
        normalized
    } else if normalized.contains('*') || normalized.contains('/') {
        format!("!{}", normalized)
    } else {
        format!("!**/{}/**", normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::tempdir;

    #[derive(Debug, Deserialize)]
    struct RealProjectGateSuite {
        projects: Vec<RealProjectGateProject>,
    }

    #[derive(Debug, Deserialize)]
    struct RealProjectGateProject {
        name: String,
        language: String,
        root: String,
        queries: Vec<RealProjectGateQuery>,
        #[serde(default)]
        exclude_paths: Vec<String>,
        #[serde(alias = "min_hybrid_recall_at_5")]
        min_accurate_recall_at_5: f64,
        #[serde(alias = "max_query_p95_ms")]
        max_balanced_query_p95_ms: u64,
        #[serde(default = "default_accurate_query_p95_ms")]
        max_accurate_query_p95_ms: u64,
    }

    #[derive(Debug, Deserialize)]
    struct RealProjectGateQuery {
        id: String,
        kind: String,
        query: String,
        expected_paths: Vec<String>,
    }

    fn default_accurate_query_p95_ms() -> u64 {
        3_000
    }

    async fn wait_for_reranker_ready(directory: &Path) {
        reranker::ensure_started(directory);
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let snapshot = reranker::runtime_snapshot(directory);
            match snapshot.phase {
                reranker::RuntimePhase::Ready => return,
                reranker::RuntimePhase::Error => {
                    panic!(
                        "准确模式模型预热失败: {}",
                        snapshot.error.unwrap_or_else(|| "未知错误".to_string())
                    );
                }
                reranker::RuntimePhase::Missing => {
                    panic!("准确模式模型资产或共享 ONNX Runtime 尚未就绪");
                }
                reranker::RuntimePhase::Loading => {}
            }
            assert!(Instant::now() < deadline, "准确模式模型预热超过 180 秒");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn result_paths(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| line.strip_prefix("Path: "))
            .map(|path| path.replace('\\', "/").to_ascii_lowercase())
            .collect()
    }

    fn paths_contain_expected(paths: &[String], expected_paths: &[String]) -> bool {
        expected_paths.iter().any(|expected| {
            let expected = expected.replace('\\', "/").to_ascii_lowercase();
            paths.iter().any(|path| path.ends_with(&expected))
        })
    }

    fn percentile(values: &[u64], percent: usize) -> u64 {
        if values.is_empty() {
            return 0;
        }
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        let index = ((sorted.len() - 1) * percent).div_ceil(100);
        sorted[index.min(sorted.len() - 1)]
    }

    fn sqlite_storage_bytes(path: &Path) -> u64 {
        let wal_path = PathBuf::from(format!("{}-wal", path.display()));
        [path, wal_path.as_path()]
            .iter()
            .filter_map(|value| fs::metadata(value).ok())
            .map(|metadata| metadata.len())
            .sum()
    }

    fn fuse_with_test_weights(
        lexical_hits: Vec<SearchHit>,
        semantic_hits: Vec<semantic::SemanticHit>,
        query: &str,
        terms: &[String],
        max_results: usize,
        lexical_weight: f64,
        semantic_weight: f64,
    ) -> Vec<SearchHit> {
        const RRF_K: f64 = 60.0;
        let mut merged: HashMap<(String, usize, usize), SearchHit> = HashMap::new();
        for (rank, mut hit) in lexical_hits.into_iter().enumerate() {
            hit.fusion_score = lexical_weight / (RRF_K + rank as f64 + 1.0);
            merged.insert(
                (hit.relative_path.clone(), hit.start_line, hit.end_line),
                hit,
            );
        }
        for (rank, semantic_hit) in semantic_hits.into_iter().enumerate() {
            let key = (
                semantic_hit.relative_path.clone(),
                semantic_hit.start_line,
                semantic_hit.end_line,
            );
            let hit = merged.entry(key).or_insert_with(|| {
                score_hit(
                    semantic_hit.relative_path.clone(),
                    semantic_hit.start_line,
                    semantic_hit.end_line,
                    semantic_hit.excerpt.clone(),
                    0.0,
                    query,
                    terms,
                )
            });
            hit.semantic_score = Some(semantic_hit.score);
            hit.fusion_score += semantic_weight / (RRF_K + rank as f64 + 1.0);
        }
        let mut hits = merged.into_values().collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .fusion_score
                .total_cmp(&left.fusion_score)
                .then_with(|| right.exact_match.cmp(&left.exact_match))
                .then_with(|| right.coverage.cmp(&left.coverage))
                .then_with(|| left.relative_path.cmp(&right.relative_path))
                .then_with(|| left.start_line.cmp(&right.start_line))
        });
        hits.truncate(max_results.max(1));
        hits
    }

    fn fuse_paths_with_test_weights(
        lexical_hits: &[SearchHit],
        semantic_ranking: &[(String, f32)],
        candidate_limit: usize,
        max_results: usize,
        lexical_weight: f64,
        semantic_weight: f64,
    ) -> Vec<String> {
        const RRF_K: f64 = 60.0;
        let mut scores = HashMap::<String, f64>::new();
        let mut lexical_paths = HashSet::new();
        for (rank, hit) in lexical_hits.iter().take(candidate_limit).enumerate() {
            let path = hit.relative_path.replace('\\', "/").to_ascii_lowercase();
            if lexical_paths.insert(path.clone()) {
                *scores.entry(path).or_default() += lexical_weight / (RRF_K + rank as f64 + 1.0);
            }
        }
        let mut semantic_paths = HashSet::new();
        for (rank, (path, _)) in semantic_ranking.iter().take(candidate_limit).enumerate() {
            let path = path.replace('\\', "/").to_ascii_lowercase();
            if semantic_paths.insert(path.clone()) {
                *scores.entry(path).or_default() += semantic_weight / (RRF_K + rank as f64 + 1.0);
            }
        }
        let mut ranked = scores.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        ranked
            .into_iter()
            .take(max_results.max(1))
            .map(|(path, _)| path)
            .collect()
    }

    #[test]
    fn changing_index_directory_replaces_cached_project_instance() {
        let temp = tempdir().expect("索引目录切换测试环境应创建成功");
        let root = temp.path().join("project");
        let first_dir = temp.path().join("first-index");
        let second_dir = temp.path().join("second-index");
        fs::create_dir_all(&root).expect("索引目录切换测试项目应创建成功");

        let first = project_index(&root, &first_dir).expect("首个索引实例应创建成功");
        fs::write(&first.db_path, b"keep-old-index").expect("旧索引占位文件应写入成功");
        let first_again = project_index(&root, &first_dir).expect("同目录索引实例应复用");
        assert!(Arc::ptr_eq(&first, &first_again));

        let second = project_index(&root, &second_dir).expect("新目录索引实例应创建成功");
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.db_path.parent(), Some(second_dir.as_path()));
        assert_eq!(
            fs::read(&first.db_path).expect("切换后旧索引文件应保留"),
            b"keep-old-index"
        );
    }

    #[test]
    fn query_terms_cover_identifiers_and_chinese_bigrams() {
        let terms = extract_query_terms("ScopeWorkspace scope_name 手机号验证码");
        assert!(terms.contains(&"scopeworkspace".to_string()));
        assert!(terms.contains(&"scope".to_string()));
        assert!(terms.contains(&"workspace".to_string()));
        assert!(terms.contains(&"手机".to_string()));
        assert!(terms.contains(&"验证码".to_string()));
    }

    #[test]
    fn long_chinese_query_keeps_trailing_code_identifier() {
        let terms = extract_query_terms(
            "请在整个大型项目中查找手机号验证码登录实现与权限路由链路 ScopeWorkspace",
        );
        assert!(terms.contains(&"scopeworkspace".to_string()));
    }

    #[test]
    fn weighted_rrf_promotes_chunks_recalled_by_both_rankers() {
        let terms = vec!["workspace".to_string()];
        let lexical = vec![
            score_hit(
                "src/lexical.rs".to_string(),
                1,
                10,
                "fn workspace() {}".to_string(),
                -2.0,
                "workspace",
                &terms,
            ),
            score_hit(
                "src/shared.rs".to_string(),
                1,
                10,
                "fn workspace_state() {}".to_string(),
                -1.0,
                "workspace",
                &terms,
            ),
        ];
        let semantic = vec![
            semantic::SemanticHit {
                relative_path: "src/shared.rs".to_string(),
                start_line: 1,
                end_line: 10,
                excerpt: "fn workspace_state() {}".to_string(),
                score: 0.8,
            },
            semantic::SemanticHit {
                relative_path: "src/semantic.rs".to_string(),
                start_line: 1,
                end_line: 10,
                excerpt: "fn current_scope() {}".to_string(),
                score: 0.7,
            },
        ];

        let fused = fuse_hits(lexical, semantic, "workspace", &terms, 3);
        assert_eq!(fused[0].relative_path, "src/shared.rs");
        assert_eq!(fused[0].semantic_score, Some(0.8));
    }

    #[test]
    fn semantic_only_results_apply_code_specific_threshold() {
        let semantic = vec![
            semantic::SemanticHit {
                relative_path: "src/strong.rs".to_string(),
                start_line: 1,
                end_line: 10,
                excerpt: "fn related_behavior() {}".to_string(),
                score: semantic::SEMANTIC_ONLY_THRESHOLD + 0.05,
            },
            semantic::SemanticHit {
                relative_path: "src/weak.rs".to_string(),
                start_line: 1,
                end_line: 10,
                excerpt: "fn unrelated_behavior() {}".to_string(),
                score: semantic::SEMANTIC_ONLY_THRESHOLD - 0.05,
            },
        ];

        let fused = fuse_hits(Vec::new(), semantic, "related behavior", &[], 5);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].relative_path, "src/strong.rs");
    }

    #[test]
    fn accurate_candidates_keep_top_thirty_two_distinct_files() {
        let retrieval_candidates = (0..40)
            .map(|index| {
                let path = if index < 8 {
                    "src/repeated.rs".to_string()
                } else {
                    format!("src/file_{index}.rs")
                };
                score_hit(
                    path,
                    index + 1,
                    index + 10,
                    format!("fn candidate_{index}() {{}}"),
                    index as f64,
                    "intent",
                    &[],
                )
            })
            .collect::<Vec<_>>();

        let candidates = select_accurate_rerank_candidates(retrieval_candidates);
        assert_eq!(candidates.len(), ACCURATE_RERANK_LIMIT);
        let unique = candidates
            .iter()
            .map(|hit| &hit.relative_path)
            .collect::<HashSet<_>>();
        assert_eq!(unique.len(), candidates.len());
        assert_eq!(candidates[0].relative_path, "src/repeated.rs");
        assert_eq!(candidates[1].relative_path, "src/file_8.rs");
    }

    #[test]
    fn accurate_ranking_keeps_exact_lexical_top_one() {
        let exact = score_hit(
            "src/exact.rs".to_string(),
            1,
            10,
            "fn ExactHandler() {}".to_string(),
            -10.0,
            "ExactHandler",
            &["exacthandler".to_string()],
        );
        assert!(exact.exact_match);
        let other = score_hit(
            "src/semantic.rs".to_string(),
            1,
            10,
            "fn inferred_intent() {}".to_string(),
            0.0,
            "ExactHandler",
            &[],
        );
        let duplicate_path = score_hit(
            "src/exact.rs".to_string(),
            20,
            30,
            "fn secondary_exact_chunk() {}".to_string(),
            0.0,
            "ExactHandler",
            &[],
        );
        let ranked = apply_reranker_ranking(
            vec![exact.clone(), duplicate_path, other],
            vec![
                reranker::RerankMatch {
                    index: 2,
                    score: 0.9,
                },
                reranker::RerankMatch {
                    index: 1,
                    score: 0.5,
                },
                reranker::RerankMatch {
                    index: 0,
                    score: 0.1,
                },
            ],
            Some(exact),
            3,
        );
        assert_eq!(ranked[0].relative_path, "src/exact.rs");
        assert_eq!(ranked[1].relative_path, "src/semantic.rs");
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn reranker_context_keeps_head_and_tail_with_a_bounded_length() {
        let source = format!(
            "HEAD{}MIDDLE{}TAIL",
            "x".repeat(RERANKER_CONTEXT_CHARS),
            "y".repeat(RERANKER_CONTEXT_CHARS)
        );
        let bounded = bounded_reranker_context(&source);
        assert!(!bounded.is_empty());
        assert!(bounded.contains("HEAD"));
        assert!(bounded.contains("TAIL"));
        assert!(bounded.contains("[context-truncated]"));
        assert!(bounded.chars().count() <= RERANKER_CONTEXT_CHARS + 32);
    }

    #[test]
    fn reranker_input_diagnostics_reports_candidate_and_length_distribution() {
        let documents = vec![
            "a".repeat(10),
            "b".repeat(20),
            "c".repeat(30),
            format!("{}[context-truncated]{}", "d".repeat(10), "e".repeat(30)),
        ];
        let diagnostics = summarize_reranker_inputs(&documents);

        assert_eq!(diagnostics.candidate_count, 4);
        assert_eq!(diagnostics.batch_count, 1);
        assert_eq!(diagnostics.truncated_candidate_count, 1);
        assert_eq!(diagnostics.min_chars, 10);
        assert_eq!(diagnostics.p50_chars, 20);
        assert_eq!(diagnostics.p95_chars, 30);
        assert_eq!(diagnostics.max_chars, documents[3].chars().count());
        assert_eq!(
            diagnostics.total_chars,
            documents
                .iter()
                .map(|value| value.chars().count())
                .sum::<usize>()
        );
    }

    #[test]
    fn fts5_index_supports_warm_multi_keyword_search_and_incremental_update() {
        let temp = tempdir().expect("临时项目应创建成功");
        let root = temp.path().join("project");
        fs::create_dir_all(root.join("src")).expect("源码目录应创建成功");
        let source = root.join("src").join("scope_workspace.rs");
        fs::write(
            &source,
            "pub struct ScopeWorkspace;\nfn append_current_options(scope_name: &str) {}\n",
        )
        .expect("测试源码应写入成功");
        let index = ProjectIndex::new(root.clone(), temp.path().join("index.sqlite3"));

        let first_counts = sync_index(&index, &[]).expect("首次索引应成功");
        assert_eq!(first_counts.0, 1);
        let terms = extract_query_terms("ScopeWorkspace scopeName appendCurrentOptions");
        let hits =
            query_index(&index.db_path, "ScopeWorkspace", &terms, 10).expect("热索引查询应成功");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].relative_path, "src/scope_workspace.rs");

        std::thread::sleep(Duration::from_millis(2));
        fs::write(&source, "pub struct RenamedWorkspace;\n").expect("测试源码应更新成功");
        sync_index(&index, &[]).expect("增量索引应成功");
        let renamed = query_index(
            &index.db_path,
            "RenamedWorkspace",
            &extract_query_terms("RenamedWorkspace"),
            10,
        )
        .expect("更新后的索引查询应成功");
        assert_eq!(renamed.len(), 1);
    }

    #[test]
    fn workspace_status_does_not_archive_legacy_parent_index() {
        let temp = tempdir().expect("应创建临时目录");
        let root = temp.path().join("workspace");
        fs::create_dir_all(root.join("server/.git")).expect("应创建工作区子项目");
        let canonical_root = root.canonicalize().expect("工作区路径应可规范化");
        let index_dir = temp.path().join("indexes");
        fs::create_dir_all(&index_dir).expect("应创建索引目录");
        let parent_db = index_dir.join(format!("{}.sqlite3", project_hash(&canonical_root)));
        fs::write(&parent_db, b"legacy-parent-index").expect("应写入遗留父索引");

        let status = workspace_status(
            &normalize_path(&canonical_root),
            Vec::new(),
            index_dir.clone(),
            LocalSemanticSettings {
                mode: LocalSemanticMode::Off,
                model_dir: crate::mcp::embedding::default_model_dir(),
                reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
            },
        )
        .expect("工作区状态读取应成功");

        assert!(status.is_workspace);
        assert!(parent_db.exists());
        assert!(!index_dir.join("archive").exists());
    }

    #[test]
    fn status_inspection_does_not_migrate_existing_database_schema() {
        let temp = tempdir().expect("应创建状态只读测试目录");
        let root = temp.path().join("project");
        fs::create_dir_all(&root).expect("应创建状态只读测试项目");
        let canonical_root = root.canonicalize().expect("项目路径应可规范化");
        let index_dir = temp.path().join("indexes");
        fs::create_dir_all(&index_dir).expect("应创建状态只读索引目录");
        let db_path = index_dir.join(format!("{}.sqlite3", project_hash(&canonical_root)));
        let connection = Connection::open(&db_path).expect("应创建最小旧版索引");
        connection
            .execute_batch(
                "CREATE TABLE files (
                    path TEXT PRIMARY KEY,
                    modified_ns INTEGER NOT NULL,
                    size INTEGER NOT NULL
                );
                CREATE VIRTUAL TABLE chunks USING fts5(
                    path UNINDEXED,
                    start_line UNINDEXED,
                    end_line UNINDEXED,
                    search_text,
                    content UNINDEXED
                );",
            )
            .expect("应创建旧版词法索引表");
        drop(connection);

        let inspected = status(
            &normalize_path(&canonical_root),
            index_dir,
            LocalSemanticSettings {
                mode: LocalSemanticMode::Off,
                model_dir: crate::mcp::embedding::default_model_dir(),
                reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
            },
        )
        .expect("状态查询应成功");

        assert_eq!(inspected.state, "ready");
        let connection = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("应只读复查旧版索引");
        let semantic_table_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunk_vectors'",
                [],
                |row| row.get(0),
            )
            .expect("应检查语义表是否存在");
        assert_eq!(semantic_table_count, 0);
    }

    #[tokio::test]
    async fn pending_index_changes_use_current_files_instead_of_stale_fts5() {
        let temp = tempdir().expect("即时搜索测试目录应创建成功");
        let root = temp.path().join("project");
        fs::create_dir_all(&root).expect("即时搜索测试项目应创建成功");
        let source = root.join("search_state.rs");
        fs::write(&source, "pub struct OldIndexValue;\n").expect("旧索引源码应写入成功");
        let index = Arc::new(ProjectIndex::new(
            root.clone(),
            temp.path().join("pending.sqlite3"),
        ));
        sync_now(
            Arc::clone(&index),
            Vec::new(),
            LocalSemanticSettings {
                mode: LocalSemanticMode::Off,
                model_dir: crate::mcp::embedding::default_model_dir(),
                reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
            },
        )
        .await
        .expect("旧内容索引应建立成功");

        fs::write(&source, "pub struct CurrentFileValue;\n").expect("当前源码应写入成功");
        index.dirty.store(true, Ordering::Release);
        // 模拟已有增量同步任务，避免测试结束后遗留后台任务。
        index.sync_running.store(true, Ordering::Release);
        let output = search_with_index(
            LocalSearchOptions {
                project_root: root.clone(),
                query: "CurrentFileValue".to_string(),
                max_results: 5,
                exclude_paths: Vec::new(),
                index_dir: temp.path().join("indexes"),
                semantic: LocalSemanticSettings {
                    mode: LocalSemanticMode::Off,
                    model_dir: crate::mcp::embedding::default_model_dir(),
                    reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
                },
            },
            root,
            Arc::clone(&index),
            false,
        )
        .await
        .expect("待同步状态应使用即时搜索");
        index.sync_running.store(false, Ordering::Release);

        assert_ne!(output.engine, "fts5");
        assert_eq!(output.hit_count, 1);
        assert!(output.text.contains("CurrentFileValue"));
        assert!(!output.degraded);
        assert!(output.fallback_reason.is_none());
        assert!(output
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("文件变更")));
    }

    #[tokio::test]
    async fn semantic_sync_keeps_ready_fts5_available() {
        let temp = tempdir().expect("语义同步测试目录应创建成功");
        let root = temp.path().join("project");
        fs::create_dir_all(&root).expect("语义同步测试项目应创建成功");
        fs::write(
            root.join("search_state.rs"),
            "pub struct LexicalIndexRemainsReady;\n",
        )
        .expect("测试源码应写入成功");
        let index = Arc::new(ProjectIndex::new(
            root.clone(),
            temp.path().join("semantic-sync.sqlite3"),
        ));
        sync_now(
            Arc::clone(&index),
            Vec::new(),
            LocalSemanticSettings {
                mode: LocalSemanticMode::Off,
                model_dir: crate::mcp::embedding::default_model_dir(),
                reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
            },
        )
        .await
        .expect("词法索引应建立成功");
        refresh_profile(&index, &[]);
        index.dirty.store(false, Ordering::Release);
        index.sync_running.store(true, Ordering::Release);
        index.lexical_sync_running.store(false, Ordering::Release);
        index
            .semantic_state
            .store(SEMANTIC_SYNCING, Ordering::Release);

        let output = search_with_index(
            LocalSearchOptions {
                project_root: root.clone(),
                query: "LexicalIndexRemainsReady".to_string(),
                max_results: 5,
                exclude_paths: Vec::new(),
                index_dir: temp.path().join("indexes"),
                semantic: LocalSemanticSettings {
                    mode: LocalSemanticMode::Off,
                    model_dir: crate::mcp::embedding::default_model_dir(),
                    reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
                },
            },
            root,
            Arc::clone(&index),
            false,
        )
        .await
        .expect("语义同步期间词法查询应可用");
        index.sync_running.store(false, Ordering::Release);

        assert_eq!(output.engine, "fts5");
        assert_eq!(output.hit_count, 1);
        assert!(output.notice.is_none());
        assert!(output.fallback_reason.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "由 phase2 门禁脚本传入已批准的真实项目与脱敏查询"]
    async fn phase2_gate_real_projects_from_env() {
        let suite_json = std::env::var("SANSHU_SOU_REAL_GATE_JSON")
            .expect("门禁脚本应提供 SANSHU_SOU_REAL_GATE_JSON");
        let suite = serde_json::from_str::<RealProjectGateSuite>(&suite_json)
            .expect("真实项目门禁配置应为有效 JSON");
        assert!(!suite.projects.is_empty());

        let model_dir = std::env::var_os("SANSHU_SOU_GATE_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(crate::mcp::embedding::default_model_dir);
        assert!(
            crate::mcp::embedding::assets_have_expected_sizes(&model_dir),
            "真实项目门禁需要本机固定 BGE 与 ONNX Runtime 资产"
        );
        let reranker_dir = std::env::var_os("SANSHU_SOU_GATE_RERANKER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(crate::config::default_sou_reranker_model_dir);
        assert!(
            reranker::assets_have_expected_sizes(&reranker_dir),
            "准确模式门禁需要本机固定 BGE-reranker-base 资产"
        );

        let mut gate_failures = Vec::new();
        let mut release_probe = None;
        for project in suite.projects {
            assert!(!project.queries.is_empty());
            let root = PathBuf::from(&project.root)
                .canonicalize()
                .expect("真实项目路径应存在");
            let index_dir = std::env::var_os("SANSHU_SOU_GATE_INDEX_DIR")
                .map(PathBuf::from)
                .expect("门禁脚本应提供 SANSHU_SOU_GATE_INDEX_DIR");
            fs::create_dir_all(&index_dir).expect("应创建真实项目门禁索引目录");
            let db_path = index_dir.join(format!("{}.sqlite3", project.name));
            let index = Arc::new(ProjectIndex::new(root.clone(), db_path));
            let semantic_settings = LocalSemanticSettings {
                mode: LocalSemanticMode::Balanced,
                model_dir: model_dir.clone(),
                reranker_model_dir: reranker_dir.clone(),
            };
            refresh_profile(&index, &project.exclude_paths);

            println!(
                "SOU_PHASE2_GATE_PROGRESS project={} stage=indexing",
                project.name
            );
            let index_started = Instant::now();
            sync_now(
                Arc::clone(&index),
                project.exclude_paths.clone(),
                semantic_settings.clone(),
            )
            .await
            .expect("真实项目 lexical/semantic 索引应建立成功");
            let index_duration_ms = index_started.elapsed().as_millis() as u64;
            let status = index.status(&semantic_settings);
            assert_eq!(status.state, "ready");
            assert_eq!(status.semantic_state, "ready");
            assert_eq!(status.semantic_pending_chunks, 0);
            assert_eq!(status.semantic_indexed_chunks, status.indexed_chunks);
            assert!(status.indexed_chunks > 0);

            let mut lexical_hits = 0usize;
            let mut hybrid_hits = 0usize;
            let mut exact_total = 0usize;
            let mut lexical_exact_top_one = 0usize;
            let mut hybrid_exact_top_one = 0usize;
            let mut hybrid_durations = Vec::new();
            let mut query_results = Vec::new();
            let mut equal_rrf_hits = 0usize;
            for query in &project.queries {
                let lexical = search_with_index(
                    LocalSearchOptions {
                        project_root: root.clone(),
                        query: query.query.clone(),
                        max_results: 5,
                        exclude_paths: project.exclude_paths.clone(),
                        index_dir: index_dir.clone(),
                        semantic: LocalSemanticSettings {
                            mode: LocalSemanticMode::Off,
                            model_dir: model_dir.clone(),
                            reranker_model_dir: reranker_dir.clone(),
                        },
                    },
                    root.clone(),
                    Arc::clone(&index),
                    false,
                )
                .await
                .expect("真实项目 lexical 查询应成功");
                let hybrid = search_with_index(
                    LocalSearchOptions {
                        project_root: root.clone(),
                        query: query.query.clone(),
                        max_results: 5,
                        exclude_paths: project.exclude_paths.clone(),
                        index_dir: index_dir.clone(),
                        semantic: semantic_settings.clone(),
                    },
                    root.clone(),
                    Arc::clone(&index),
                    false,
                )
                .await
                .expect("真实项目 hybrid 查询应成功");

                assert_eq!(lexical.engine, "fts5");
                assert_eq!(hybrid.engine, "fts5+bge");
                assert_eq!(hybrid.semantic_state, "ready");
                assert!(hybrid.fallback_reason.is_none());
                let lexical_paths = result_paths(&lexical.text);
                let hybrid_paths = result_paths(&hybrid.text);
                let root_prefix = normalize_path(&root).to_ascii_lowercase();
                let lexical_relative_paths = lexical_paths
                    .iter()
                    .map(|path| {
                        path.strip_prefix(&root_prefix)
                            .unwrap_or(path)
                            .trim_start_matches('/')
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                let hybrid_relative_paths = hybrid_paths
                    .iter()
                    .map(|path| {
                        path.strip_prefix(&root_prefix)
                            .unwrap_or(path)
                            .trim_start_matches('/')
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                let lexical_hit = paths_contain_expected(&lexical_paths, &query.expected_paths);
                let hybrid_hit = paths_contain_expected(&hybrid_paths, &query.expected_paths);
                lexical_hits += usize::from(lexical_hit);
                hybrid_hits += usize::from(hybrid_hit);
                hybrid_durations.push(hybrid.duration_ms);

                let terms = extract_query_terms(&query.query);
                let lexical_candidates = query_index(&index.db_path, &query.query, &terms, 50)
                    .expect("真实项目 lexical 诊断候选应读取成功");
                let (semantic_candidates, embedding_ms, scan_ms) =
                    semantic::search_timed_for_test(&index.db_path, &model_dir, &query.query, 25)
                        .await
                        .expect("真实项目 semantic 分段诊断应成功");
                let semantic_relative_paths = semantic_candidates
                    .iter()
                    .map(|hit| hit.relative_path.replace('\\', "/").to_ascii_lowercase())
                    .collect::<Vec<_>>();
                let semantic_expected_rank = semantic_relative_paths
                    .iter()
                    .position(|path| {
                        paths_contain_expected(std::slice::from_ref(path), &query.expected_paths)
                    })
                    .map(|rank| rank + 1);
                let full_semantic_ranking =
                    semantic::rank_paths_for_test(&index.db_path, &model_dir, &query.query)
                        .await
                        .expect("真实项目 semantic 全量排名诊断应成功");
                let full_semantic_expected =
                    full_semantic_ranking
                        .iter()
                        .enumerate()
                        .find(|(_, (path, _))| {
                            let path = path.replace('\\', "/").to_ascii_lowercase();
                            paths_contain_expected(
                                std::slice::from_ref(&path),
                                &query.expected_paths,
                            )
                        });
                let full_semantic_expected_rank = full_semantic_expected.map(|(rank, _)| rank + 1);
                let full_semantic_expected_score =
                    full_semantic_expected.map(|(_, (_, score))| *score);
                let lexical_candidate_paths = lexical_candidates
                    .iter()
                    .map(|hit| hit.relative_path.replace('\\', "/").to_ascii_lowercase())
                    .collect::<Vec<_>>();
                let lexical_expected_rank = lexical_candidate_paths
                    .iter()
                    .position(|path| {
                        paths_contain_expected(std::slice::from_ref(path), &query.expected_paths)
                    })
                    .map(|rank| rank + 1);
                let path_rrf_65_35 = fuse_paths_with_test_weights(
                    &lexical_candidates,
                    &full_semantic_ranking,
                    50,
                    5,
                    0.65,
                    0.35,
                );
                let path_rrf_50_50 = fuse_paths_with_test_weights(
                    &lexical_candidates,
                    &full_semantic_ranking,
                    50,
                    5,
                    0.5,
                    0.5,
                );
                let equal_rrf = fuse_with_test_weights(
                    lexical_candidates.iter().take(25).cloned().collect(),
                    semantic_candidates,
                    &query.query,
                    &terms,
                    5,
                    0.5,
                    0.5,
                );
                let equal_rrf_paths = equal_rrf
                    .iter()
                    .map(|hit| hit.relative_path.replace('\\', "/").to_ascii_lowercase())
                    .collect::<Vec<_>>();
                let equal_rrf_hit = paths_contain_expected(&equal_rrf_paths, &query.expected_paths);
                equal_rrf_hits += usize::from(equal_rrf_hit);

                let mut lexical_top_one = false;
                let mut hybrid_top_one = false;
                if query.kind == "exact_identifier" {
                    exact_total += 1;
                    lexical_top_one = lexical_paths.first().is_some_and(|path| {
                        paths_contain_expected(std::slice::from_ref(path), &query.expected_paths)
                    });
                    hybrid_top_one = hybrid_paths.first().is_some_and(|path| {
                        paths_contain_expected(std::slice::from_ref(path), &query.expected_paths)
                    });
                    lexical_exact_top_one += usize::from(lexical_top_one);
                    hybrid_exact_top_one += usize::from(hybrid_top_one);
                }

                query_results.push(serde_json::json!({
                    "id": query.id,
                    "kind": query.kind,
                    "lexical_hit_at_5": lexical_hit,
                    "hybrid_hit_at_5": hybrid_hit,
                    "lexical_top_one": lexical_top_one,
                    "hybrid_top_one": hybrid_top_one,
                    "lexical_duration_ms": lexical.duration_ms,
                    "hybrid_duration_ms": hybrid.duration_ms,
                    "semantic_top_score": hybrid.semantic_top_score,
                    "semantic_embedding_ms": embedding_ms,
                    "semantic_scan_ms": scan_ms,
                    "semantic_expected_rank": semantic_expected_rank,
                    "semantic_full_expected_rank": full_semantic_expected_rank,
                    "semantic_full_expected_score": full_semantic_expected_score,
                    "semantic_total_candidates": full_semantic_ranking.len(),
                    "lexical_expected_rank_at_50": lexical_expected_rank,
                    "path_rrf_65_35_hit_at_5": paths_contain_expected(&path_rrf_65_35, &query.expected_paths),
                    "path_rrf_65_35_top_paths": path_rrf_65_35,
                    "path_rrf_50_50_hit_at_5": paths_contain_expected(&path_rrf_50_50, &query.expected_paths),
                    "path_rrf_50_50_top_paths": path_rrf_50_50,
                    "semantic_top_paths": semantic_relative_paths.iter().take(10).collect::<Vec<_>>(),
                    "equal_rrf_hit_at_5": equal_rrf_hit,
                    "equal_rrf_top_paths": equal_rrf_paths,
                    "lexical_top_paths": lexical_relative_paths,
                    "hybrid_top_paths": hybrid_relative_paths,
                }));
            }

            println!(
                "SOU_PHASE2_GATE_PROGRESS project={} stage=reranker_warmup",
                project.name
            );
            wait_for_reranker_ready(&reranker_dir).await;
            let accurate_settings = LocalSemanticSettings {
                mode: LocalSemanticMode::Accurate,
                model_dir: model_dir.clone(),
                reranker_model_dir: reranker_dir.clone(),
            };
            let warmup = search_with_index(
                LocalSearchOptions {
                    project_root: root.clone(),
                    query: project.queries[0].query.clone(),
                    max_results: 5,
                    exclude_paths: project.exclude_paths.clone(),
                    index_dir: index_dir.clone(),
                    semantic: accurate_settings.clone(),
                },
                root.clone(),
                Arc::clone(&index),
                false,
            )
            .await
            .expect("真实项目 accurate 预热查询应成功");
            // 中文说明：门禁失败时保留完整阶段状态，便于区分预算耗尽与运行时未就绪。
            println!(
                "SOU_PHASE2_ACCURATE_WARMUP={}",
                serde_json::json!({
                    "project": project.name,
                    "engine": warmup.engine,
                    "semantic_mode": warmup.semantic_mode,
                    "semantic_state": warmup.semantic_state,
                    "reranker_state": warmup.reranker_state,
                    "reranker_duration_ms": warmup.reranker_duration_ms,
                    "reranker_input": warmup.reranker_input_diagnostics.as_ref(),
                    "fallback_reason": warmup.fallback_reason,
                    "duration_ms": warmup.duration_ms,
                    "fusion": warmup.fusion,
                })
            );
            assert_eq!(
                warmup.engine,
                "fts5+bge+reranker",
                "accurate 预热状态: mode={}, semantic_state={}, reranker_state={:?}, fallback_reason={:?}, duration_ms={}, fusion={:?}",
                warmup.semantic_mode,
                warmup.semantic_state,
                warmup.reranker_state,
                warmup.fallback_reason,
                warmup.duration_ms,
                warmup.fusion
            );
            assert_eq!(warmup.reranker_state.as_deref(), Some("ready"));

            let mut accurate_hits = 0usize;
            let mut accurate_exact_top_one = 0usize;
            let mut accurate_durations = Vec::new();
            let mut reranker_durations = Vec::new();
            let mut accurate_query_results = Vec::new();
            for query in &project.queries {
                let accurate = search_with_index(
                    LocalSearchOptions {
                        project_root: root.clone(),
                        query: query.query.clone(),
                        max_results: 5,
                        exclude_paths: project.exclude_paths.clone(),
                        index_dir: index_dir.clone(),
                        semantic: accurate_settings.clone(),
                    },
                    root.clone(),
                    Arc::clone(&index),
                    false,
                )
                .await
                .expect("真实项目 accurate 查询应成功");
                assert_eq!(accurate.engine, "fts5+bge+reranker");
                assert_eq!(accurate.semantic_mode, "accurate");
                assert_eq!(accurate.reranker_state.as_deref(), Some("ready"));
                assert_eq!(accurate.fusion.as_deref(), Some(ACCURATE_FUSION_NAME));
                assert!(accurate.fallback_reason.is_none());

                let accurate_paths = result_paths(&accurate.text);
                let accurate_hit = paths_contain_expected(&accurate_paths, &query.expected_paths);
                accurate_hits += usize::from(accurate_hit);
                accurate_durations.push(accurate.duration_ms);
                if let Some(duration) = accurate.reranker_duration_ms {
                    reranker_durations.push(duration);
                }
                let accurate_top_one = query.kind == "exact_identifier"
                    && accurate_paths.first().is_some_and(|path| {
                        paths_contain_expected(std::slice::from_ref(path), &query.expected_paths)
                    });
                accurate_exact_top_one += usize::from(accurate_top_one);
                let root_prefix = normalize_path(&root).to_ascii_lowercase();
                let accurate_relative_paths = accurate_paths
                    .iter()
                    .map(|path| {
                        path.strip_prefix(&root_prefix)
                            .unwrap_or(path)
                            .trim_start_matches('/')
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                accurate_query_results.push(serde_json::json!({
                    "id": query.id,
                    "kind": query.kind,
                    "hit_at_5": accurate_hit,
                    "top_one": accurate_top_one,
                    "duration_ms": accurate.duration_ms,
                    "semantic_top_score": accurate.semantic_top_score,
                    "reranker_duration_ms": accurate.reranker_duration_ms,
                    "reranker_top_score": accurate.reranker_top_score,
                    "reranker_input": accurate.reranker_input_diagnostics.as_ref(),
                    "top_paths": accurate_relative_paths,
                }));
            }

            let query_count = project.queries.len();
            let lexical_recall = lexical_hits as f64 / query_count as f64;
            let hybrid_recall = hybrid_hits as f64 / query_count as f64;
            let accurate_recall = accurate_hits as f64 / query_count as f64;
            let equal_rrf_recall = equal_rrf_hits as f64 / query_count as f64;
            let p50_ms = percentile(&hybrid_durations, 50);
            let p95_ms = percentile(&hybrid_durations, 95);
            let accurate_p50_ms = percentile(&accurate_durations, 50);
            let accurate_p95_ms = percentile(&accurate_durations, 95);
            let reranker_p95_ms = percentile(&reranker_durations, 95);
            let result = serde_json::json!({
                "type": "real_project",
                "project": project.name,
                "language": project.language,
                "indexed_files": status.indexed_files,
                "indexed_chunks": status.indexed_chunks,
                "index_duration_ms": index_duration_ms,
                "index_chunks_per_second": status.indexed_chunks as f64 / (index_duration_ms.max(1) as f64 / 1000.0),
                "sqlite_bytes": sqlite_storage_bytes(&index.db_path),
                "query_count": query_count,
                "lexical_recall_at_5": lexical_recall,
                "hybrid_recall_at_5": hybrid_recall,
                "accurate_recall_at_5": accurate_recall,
                "equal_rrf_recall_at_5": equal_rrf_recall,
                "recall_delta": hybrid_recall - lexical_recall,
                "accurate_recall_delta": accurate_recall - hybrid_recall,
                "exact_identifier_count": exact_total,
                "lexical_exact_top_one": lexical_exact_top_one,
                "hybrid_exact_top_one": hybrid_exact_top_one,
                "accurate_exact_top_one": accurate_exact_top_one,
                "hybrid_query_p50_ms": p50_ms,
                "hybrid_query_p95_ms": p95_ms,
                "accurate_query_p50_ms": accurate_p50_ms,
                "accurate_query_p95_ms": accurate_p95_ms,
                "reranker_p95_ms": reranker_p95_ms,
                "queries": query_results,
                "accurate_queries": accurate_query_results,
            });
            println!("SOU_PHASE2_GATE_RESULT={result}");

            if accurate_recall < project.min_accurate_recall_at_5 {
                gate_failures.push(format!(
                    "{} accurate Recall@5 {:.3} 低于门槛 {:.3}",
                    project.name, accurate_recall, project.min_accurate_recall_at_5
                ));
            }
            if hybrid_recall + f64::EPSILON < lexical_recall {
                gate_failures.push(format!(
                    "{} hybrid Recall@5 {:.3} 低于 lexical {:.3}",
                    project.name, hybrid_recall, lexical_recall
                ));
            }
            if accurate_recall + f64::EPSILON < hybrid_recall {
                gate_failures.push(format!(
                    "{} accurate Recall@5 {:.3} 低于 hybrid {:.3}",
                    project.name, accurate_recall, hybrid_recall
                ));
            }
            if p95_ms > project.max_balanced_query_p95_ms {
                gate_failures.push(format!(
                    "{} hybrid p95 {}ms 超过门槛 {}ms",
                    project.name, p95_ms, project.max_balanced_query_p95_ms
                ));
            }
            if accurate_p95_ms > project.max_accurate_query_p95_ms {
                gate_failures.push(format!(
                    "{} accurate p95 {}ms 超过门槛 {}ms",
                    project.name, accurate_p95_ms, project.max_accurate_query_p95_ms
                ));
            }
            if exact_total > 0 && hybrid_exact_top_one < lexical_exact_top_one {
                gate_failures.push(format!(
                    "{} hybrid 精确标识符 Top1 {} 低于 lexical {}",
                    project.name, hybrid_exact_top_one, lexical_exact_top_one
                ));
            }
            if exact_total > 0 && accurate_exact_top_one < lexical_exact_top_one {
                gate_failures.push(format!(
                    "{} accurate 精确标识符 Top1 {} 低于 lexical {}",
                    project.name, accurate_exact_top_one, lexical_exact_top_one
                ));
            }
            release_probe = Some((
                root,
                index,
                project.exclude_paths,
                index_dir,
                project.queries[0].query.clone(),
            ));
        }

        let (root, index, exclude_paths, index_dir, query) =
            release_probe.expect("真实项目门禁应保留 Balanced 切换探针");
        let release_result = search_with_index(
            LocalSearchOptions {
                project_root: root.clone(),
                query,
                max_results: 5,
                exclude_paths,
                index_dir,
                semantic: LocalSemanticSettings {
                    mode: LocalSemanticMode::Balanced,
                    model_dir,
                    reranker_model_dir: reranker_dir.clone(),
                },
            },
            root,
            index,
            false,
        )
        .await
        .expect("切回 Balanced 的释放探针查询应成功");
        assert_eq!(release_result.engine, "fts5+bge");
        assert_eq!(
            reranker::runtime_snapshot(&reranker_dir).phase,
            reranker::RuntimePhase::Missing
        );
        let marker_path = std::env::var_os("SANSHU_SOU_GATE_RELEASE_MARKER")
            .map(PathBuf::from)
            .expect("门禁脚本应提供运行时释放标记路径");
        fs::write(&marker_path, b"balanced").expect("应写入运行时释放标记");
        let release_wait_seconds = std::env::var("SANSHU_SOU_GATE_RELEASE_WAIT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30);
        tokio::time::sleep(Duration::from_secs(release_wait_seconds)).await;
        println!(
            "SOU_PHASE2_RELEASE_RESULT={}",
            serde_json::json!({
                "type": "balanced_release",
                "wait_seconds": release_wait_seconds,
                "runtime_state": reranker::runtime_snapshot(&reranker_dir).phase.as_str(),
            })
        );
        assert!(
            gate_failures.is_empty(),
            "真实项目门禁未通过: {}",
            gate_failures.join("；")
        );
    }

    #[test]
    #[ignore = "由 scripts/test-sou-local-fallback.ps1 显式执行性能基准"]
    fn warm_fts5_query_p95_is_within_target_for_thousands_of_files() {
        let temp = tempdir().expect("性能测试目录应创建成功");
        let root = temp.path().join("project");
        fs::create_dir_all(root.join("src")).expect("性能测试源码目录应创建成功");
        for index in 0..2500 {
            fs::write(
                root.join("src").join(format!("module_{index}.rs")),
                format!(
                    "pub struct ScopeWorkspace{index};\nfn append_current_options_{index}(scope_name: &str) {{}}\n"
                ),
            )
            .expect("性能测试源码应写入成功");
        }
        let index = ProjectIndex::new(root, temp.path().join("bench.sqlite3"));
        sync_index(&index, &[]).expect("性能测试索引应建立成功");
        let query = "ScopeWorkspace appendCurrentOptions scopeName";
        let terms = extract_query_terms(query);

        let mut durations = Vec::new();
        for _ in 0..80 {
            let started = Instant::now();
            let hits =
                query_index(&index.db_path, query, &terms, 10).expect("性能测试热查询应成功");
            assert_eq!(hits.len(), 10);
            durations.push(started.elapsed().as_micros() as u64);
        }
        durations.sort_unstable();
        let p95_us = durations[durations.len() * 95 / 100];
        println!(
            "sou_local_benchmark files=2500 queries=80 p95_us={} target_us=50000",
            p95_us
        );
        assert!(
            p95_us <= 50_000,
            "warm FTS5 p95 超过 50ms 目标: {}us",
            p95_us
        );
    }
}
