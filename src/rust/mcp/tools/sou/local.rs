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

use super::semantic;

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

static PROJECT_INDEXES: Lazy<Mutex<HashMap<PathBuf, Arc<ProjectIndex>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub(super) struct LocalSearchOptions {
    pub project_root: PathBuf,
    pub query: String,
    pub max_results: usize,
    pub exclude_paths: Vec<String>,
    pub semantic: LocalSemanticSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalSemanticSettings {
    pub enabled: bool,
    pub model_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct LocalSearchOutput {
    pub text: String,
    pub hit_count: usize,
    pub engine: String,
    pub index_state: String,
    pub fallback_reason: Option<String>,
    pub duration_ms: u64,
    pub semantic_state: String,
    pub semantic_model: Option<String>,
    pub semantic_indexed_chunks: u64,
    pub semantic_pending_chunks: u64,
    pub semantic_top_score: Option<f32>,
    pub fusion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalIndexStatus {
    pub project_root: String,
    pub index_path: String,
    pub state: String,
    pub indexed_files: u64,
    pub indexed_chunks: u64,
    pub sync_running: bool,
    pub pending_changes: bool,
    pub last_error: Option<String>,
    pub semantic_state: String,
    pub semantic_model: Option<String>,
    pub semantic_indexed_chunks: u64,
    pub semantic_pending_chunks: u64,
    pub semantic_last_error: Option<String>,
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
            // 进程重启后先做一次元数据对账，查询仍可读取已有索引。
            dirty: Arc::new(AtomicBool::new(state == INDEX_READY)),
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
        LocalIndexStatus {
            project_root: normalize_path(&self.root),
            index_path: normalize_path(&self.db_path),
            state: self.state_name().to_string(),
            indexed_files: self.indexed_files.load(Ordering::Acquire),
            indexed_chunks: self.indexed_chunks.load(Ordering::Acquire),
            sync_running: self.sync_running.load(Ordering::Acquire),
            pending_changes: self.dirty.load(Ordering::Acquire),
            last_error: self.last_error.lock().ok().and_then(|value| value.clone()),
            semantic_state: self
                .semantic_state_name(semantic_settings.enabled)
                .to_string(),
            semantic_model: semantic_settings.enabled.then(|| semantic::model_key()),
            semantic_indexed_chunks: self.semantic_indexed_chunks.load(Ordering::Acquire),
            semantic_pending_chunks: self.semantic_pending_chunks.load(Ordering::Acquire),
            semantic_last_error: self
                .semantic_last_error
                .lock()
                .ok()
                .and_then(|value| value.clone()),
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

    let index = project_index(&root)?;
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

    let mut fallback_reason = None;
    let (mut hits, mut engine) = if index.state.load(Ordering::Acquire) == INDEX_READY {
        if index.dirty.load(Ordering::Acquire) || index.sync_running.load(Ordering::Acquire) {
            fallback_reason = Some("本地索引存在待同步变更，本次使用即时搜索".to_string());
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
            let max_results = if options.semantic.enabled {
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
    let mut fusion = None;
    if options.semantic.enabled && engine == "fts5" {
        let semantic_deadline = Instant::now() + Duration::from_secs(2);
        if !crate::mcp::embedding::assets_have_expected_sizes(&options.semantic.model_dir) {
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
                let semantic_limit = options.max_results.saturating_mul(5).min(150);
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
                        hits = fuse_hits(
                            hits,
                            semantic_hits,
                            &options.query,
                            &terms,
                            options.max_results,
                        );
                        engine = "fts5+bge".to_string();
                        fusion = Some(semantic::FUSION_NAME.to_string());
                    }
                    Err(error) => {
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
                        append_fallback(
                            &mut fallback_reason,
                            &format!("BGE 查询失败，本次保留 FTS5 结果: {}", error),
                        );
                    }
                }
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
    if engine != "fts5+bge" {
        hits.truncate(options.max_results.max(1));
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let state = index.state_name().to_string();
    let semantic_state = index
        .semantic_state_name(options.semantic.enabled)
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
        &semantic_state,
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_top_score,
        fusion.as_deref(),
    );
    Ok(LocalSearchOutput {
        text,
        hit_count: hits.len(),
        engine,
        index_state: state,
        fallback_reason,
        duration_ms,
        semantic_state,
        semantic_model: options.semantic.enabled.then(|| semantic::model_key()),
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_top_score,
        fusion,
    })
}

pub async fn rebuild(
    project_root: &str,
    exclude_paths: Vec<String>,
    semantic_settings: LocalSemanticSettings,
) -> Result<LocalIndexStatus> {
    let root = PathBuf::from(project_root)
        .canonicalize()
        .with_context(|| format!("本地索引项目路径无效: {}", project_root))?;
    let index = project_index(&root)?;
    refresh_profile(&index, &exclude_paths);
    sync_now(Arc::clone(&index), exclude_paths, semantic_settings.clone()).await?;
    Ok(index.status(&semantic_settings))
}

pub fn status(
    project_root: &str,
    semantic_settings: LocalSemanticSettings,
) -> Result<LocalIndexStatus> {
    let root = PathBuf::from(project_root)
        .canonicalize()
        .with_context(|| format!("本地索引项目路径无效: {}", project_root))?;
    Ok(project_index(&root)?.status(&semantic_settings))
}

fn project_index(root: &Path) -> Result<Arc<ProjectIndex>> {
    let mut indexes = PROJECT_INDEXES
        .lock()
        .map_err(|_| anyhow!("本地索引管理器锁已损坏"))?;
    if let Some(index) = indexes.get(root) {
        return Ok(Arc::clone(index));
    }

    let config_dir = dirs::config_dir().ok_or_else(|| anyhow!("无法定位系统配置目录"))?;
    let index_dir = config_dir.join("sanshu").join("sou-index");
    fs::create_dir_all(&index_dir).context("创建 sou 本地索引目录失败")?;
    let db_path = index_dir.join(format!("{}.sqlite3", project_hash(root)));
    let index = Arc::new(ProjectIndex::new(root.to_path_buf(), db_path));
    indexes.insert(root.to_path_buf(), Arc::clone(&index));
    Ok(index)
}

fn refresh_profile(index: &ProjectIndex, excludes: &[String]) {
    let profile = profile_hash(excludes);
    if let Ok(mut current) = index.profile_hash.lock() {
        if *current != profile {
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
    if index.state.load(Ordering::Acquire) != INDEX_READY {
        index.state.store(INDEX_BUILDING, Ordering::Release);
    }
    if semantic_settings.enabled {
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
    index.state.store(INDEX_BUILDING, Ordering::Release);
    if semantic_settings.enabled {
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
    let (files, chunks) = sync_index(index, exclude_paths)?;
    let semantic = if !semantic_settings.enabled {
        index
            .semantic_state
            .store(SEMANTIC_DISABLED, Ordering::Release);
        SemanticSyncOutcome::Disabled
    } else if !crate::mcp::embedding::assets_have_expected_sizes(&semantic_settings.model_dir) {
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
    match open_database(db_path).and_then(|connection| index_counts(&connection)) {
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
    semantic_state: &str,
    semantic_indexed_chunks: u64,
    semantic_pending_chunks: u64,
    semantic_top_score: Option<f32>,
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
        "[sou-local] engine={}, index_state={}, hits={}, duration_ms={}, semantic_state={}, semantic_indexed_chunks={}, semantic_pending_chunks={}{}{}",
        engine,
        state,
        hits.len(),
        duration_ms,
        semantic_state,
        semantic_indexed_chunks,
        semantic_pending_chunks,
        semantic_top_score
            .map(|score| format!(", semantic_top_score={:.4}", score))
            .unwrap_or_default(),
        fusion
            .map(|value| format!(", fusion={}", value))
            .unwrap_or_default()
    ));
    if let Some(reason) = fallback_reason {
        parts.push(format!("[sou-local fallback] {}", reason));
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

fn extract_query_terms(query: &str) -> Vec<String> {
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
    use tempfile::tempdir;

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
                enabled: false,
                model_dir: crate::mcp::embedding::default_model_dir(),
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
                semantic: LocalSemanticSettings {
                    enabled: false,
                    model_dir: crate::mcp::embedding::default_model_dir(),
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
