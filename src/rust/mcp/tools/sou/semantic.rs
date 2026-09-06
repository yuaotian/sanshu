//! sou Local 后端的 BGE 向量持久化、精确扫描与有界内存缓存。

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use ring::digest::{Context as ShaContext, SHA256};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::mcp::embedding;

pub(super) const FUSION_NAME: &str = "weighted_rrf_0.65_0.35_k60";
pub(super) const SEMANTIC_ONLY_THRESHOLD: f32 = 0.37;
const CACHE_PROJECT_LIMIT: usize = 2;
const CACHE_BYTE_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct SemanticHit {
    pub relative_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub excerpt: String,
    pub score: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SemanticIndexStats {
    pub indexed_chunks: u64,
    pub pending_chunks: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SemanticSyncStats {
    pub indexed_chunks: u64,
    pub pending_chunks: u64,
}

#[derive(Clone)]
struct SourceChunk {
    key: Vec<u8>,
    content_hash: Vec<u8>,
    rowid: i64,
    relative_path: String,
    start_line: usize,
    end_line: usize,
    excerpt: String,
}

struct CachedVector {
    key: Vec<u8>,
    chunk_rowid: i64,
    relative_path: String,
    start_line: usize,
    end_line: usize,
    embedding: Vec<f32>,
}

struct CacheEntry {
    db_path: PathBuf,
    generation: String,
    bytes: usize,
    vectors: Arc<Vec<CachedVector>>,
}

#[derive(Default)]
struct VectorCache {
    entries: VecDeque<CacheEntry>,
    bytes: usize,
}

static VECTOR_CACHE: Lazy<Mutex<VectorCache>> = Lazy::new(|| Mutex::new(VectorCache::default()));

pub(super) fn model_key() -> String {
    format!(
        "{}@{}:{}",
        embedding::MODEL_NAME,
        embedding::MODEL_REVISION,
        embedding::MODEL_DIMENSION
    )
}

pub(super) fn inspect(db_path: &Path) -> Result<SemanticIndexStats> {
    if !db_path.is_file() {
        return Ok(SemanticIndexStats {
            indexed_chunks: 0,
            pending_chunks: 0,
        });
    }
    // 状态查询只读取既有表，不触发 schema 迁移或 WAL 配置变更。
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("只读打开 sou 语义索引失败: {}", db_path.display()))?;
    connection.busy_timeout(Duration::from_millis(250))?;
    let total = connection.query_row("SELECT COUNT(*) FROM chunks", [], |row| {
        row.get::<_, u64>(0)
    })?;
    let indexed = connection.query_row(
        "SELECT COUNT(*) FROM chunk_vectors
         WHERE model_key = ?1 AND dimension = ?2 AND chunk_rowid IS NOT NULL",
        params![model_key(), embedding::MODEL_DIMENSION as i64],
        |row| row.get::<_, u64>(0),
    )?;
    Ok(SemanticIndexStats {
        indexed_chunks: indexed.min(total),
        pending_chunks: total.saturating_sub(indexed.min(total)),
    })
}

#[derive(Debug, Clone, Copy)]
struct SemanticSyncTuning {
    embedding_batch_size: usize,
    commit_each_batch: bool,
    dynamic_resources: bool,
}

impl Default for SemanticSyncTuning {
    fn default() -> Self {
        Self {
            embedding_batch_size: embedding::EMBEDDING_BATCH_SIZE,
            commit_each_batch: true,
            dynamic_resources: true,
        }
    }
}

pub(super) fn sync_vectors<F>(
    db_path: &Path,
    model_dir: &Path,
    on_progress: F,
) -> Result<SemanticSyncStats>
where
    F: FnMut(u64, u64),
{
    sync_vectors_with_tuning(
        db_path,
        model_dir,
        on_progress,
        SemanticSyncTuning::default(),
    )
}

fn sync_vectors_with_tuning<F>(
    db_path: &Path,
    model_dir: &Path,
    mut on_progress: F,
    tuning: SemanticSyncTuning,
) -> Result<SemanticSyncStats>
where
    F: FnMut(u64, u64),
{
    let mut connection = open_database(db_path)?;
    let chunks = load_source_chunks(&connection)?;
    let current_keys = chunks
        .iter()
        .map(|chunk| chunk.key.clone())
        .collect::<HashSet<_>>();
    let key = model_key();
    let existing = load_existing_rows(&connection, &key)?;

    {
        let transaction = connection.transaction()?;
        for stale in existing
            .keys()
            .filter(|value| !current_keys.contains(*value))
        {
            transaction.execute(
                "DELETE FROM chunk_vectors WHERE chunk_key = ?1 AND model_key = ?2",
                params![stale, key],
            )?;
        }
        transaction.execute(
            "DELETE FROM chunk_vectors WHERE model_key = ?1 AND dimension != ?2",
            params![key, embedding::MODEL_DIMENSION as i64],
        )?;
        for chunk in &chunks {
            if existing
                .get(&chunk.key)
                .is_some_and(|rowid| *rowid != Some(chunk.rowid))
            {
                transaction.execute(
                    "UPDATE chunk_vectors SET chunk_rowid = ?1
                     WHERE chunk_key = ?2 AND model_key = ?3",
                    params![chunk.rowid, chunk.key, key],
                )?;
            }
        }
        transaction.commit()?;
    }

    let mut pending = chunks
        .iter()
        .filter(|chunk| !existing.contains_key(&chunk.key))
        .cloned()
        .collect::<Vec<_>>();
    let total = chunks.len() as u64;
    let mut indexed = total.saturating_sub(pending.len() as u64);
    on_progress(indexed, pending.len() as u64);

    if tuning.commit_each_batch {
        while !pending.is_empty() {
            let batch_size = if tuning.dynamic_resources {
                embedding::prepare_semantic_batch(model_dir)
                    .map_err(|error| anyhow::anyhow!("resource: {}", error))?
            } else {
                tuning.embedding_batch_size.max(1)
            };
            let batch_count = batch_size.min(pending.len());
            let batch = pending.drain(..batch_count).collect::<Vec<_>>();
            let documents = batch
                .iter()
                .map(|chunk| format!("{}\n{}", chunk.relative_path, chunk.excerpt))
                .collect::<Vec<_>>();
            let embeddings = embedding::embed_documents_blocking_with_batch_size(
                model_dir,
                documents,
                Duration::from_secs(120),
                batch_size,
            )
            .map_err(|error| anyhow::anyhow!("{}: {}", error.state, error.message))?;
            let transaction = connection.transaction()?;
            insert_vector_batch(&transaction, &batch, embeddings, &key)?;
            transaction.commit()?;
            indexed += batch.len() as u64;
            on_progress(indexed, pending.len() as u64);
        }
    } else {
        // 中文说明：基准变体把所有向量写入一个事务，用于量化 SQLite commit 开销；生产默认仍逐批提交。
        let transaction = connection.transaction()?;
        while !pending.is_empty() {
            let batch_size = if tuning.dynamic_resources {
                embedding::prepare_semantic_batch(model_dir)
                    .map_err(|error| anyhow::anyhow!("resource: {}", error))?
            } else {
                tuning.embedding_batch_size.max(1)
            };
            let batch_count = batch_size.min(pending.len());
            let batch = pending.drain(..batch_count).collect::<Vec<_>>();
            let documents = batch
                .iter()
                .map(|chunk| format!("{}\n{}", chunk.relative_path, chunk.excerpt))
                .collect::<Vec<_>>();
            let embeddings = embedding::embed_documents_blocking_with_batch_size(
                model_dir,
                documents,
                Duration::from_secs(120),
                batch_size,
            )
            .map_err(|error| anyhow::anyhow!("{}: {}", error.state, error.message))?;
            insert_vector_batch(&transaction, &batch, embeddings, &key)?;
            indexed += batch.len() as u64;
            on_progress(indexed, pending.len() as u64);
        }
        transaction.commit()?;
    }

    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    connection.execute(
        "INSERT OR REPLACE INTO semantic_index_meta(key, value) VALUES ('generation', ?1)",
        params![generation.to_string()],
    )?;
    connection.execute(
        "INSERT OR REPLACE INTO semantic_index_meta(key, value) VALUES ('model_key', ?1)",
        params![key],
    )?;
    invalidate_cache(db_path);
    Ok(SemanticSyncStats {
        indexed_chunks: total,
        pending_chunks: 0,
    })
}

fn insert_vector_batch(
    transaction: &rusqlite::Transaction<'_>,
    batch: &[SourceChunk],
    embeddings: Vec<Vec<f32>>,
    key: &str,
) -> Result<()> {
    for (chunk, vector) in batch.iter().zip(embeddings) {
        transaction.execute(
            "INSERT OR REPLACE INTO chunk_vectors(
                chunk_key, path, start_line, end_line, content_hash, chunk_rowid,
                model_key, dimension, embedding
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                chunk.key,
                chunk.relative_path,
                chunk.start_line as i64,
                chunk.end_line as i64,
                chunk.content_hash,
                chunk.rowid,
                key,
                embedding::MODEL_DIMENSION as i64,
                encode_vector(&vector),
            ],
        )?;
    }
    Ok(())
}

pub(super) async fn search(
    db_path: &Path,
    model_dir: &Path,
    query: &str,
    fetch_limit: usize,
    wait_budget: Duration,
) -> Result<Vec<SemanticHit>, String> {
    let query_embedding = embedding::embed_query(model_dir, query, wait_budget)
        .await
        .map_err(|error| format!("{}: {}", error.state, error.message))?;
    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        search_blocking(&db_path, &query_embedding, fetch_limit.max(1))
    })
    .await
    .map_err(|error| format!("等待语义精确扫描任务失败: {}", error))?
    .map_err(|error| error.to_string())
}

#[cfg(test)]
pub(super) async fn search_timed_for_test(
    db_path: &Path,
    model_dir: &Path,
    query: &str,
    fetch_limit: usize,
) -> Result<(Vec<SemanticHit>, u64, u64), String> {
    let embedding_started = std::time::Instant::now();
    let query_embedding = embedding::embed_query(model_dir, query, Duration::from_secs(2))
        .await
        .map_err(|error| format!("{}: {}", error.state, error.message))?;
    let embedding_ms = embedding_started.elapsed().as_millis() as u64;
    let db_path = db_path.to_path_buf();
    let scan_started = std::time::Instant::now();
    let hits = tokio::task::spawn_blocking(move || {
        search_blocking(&db_path, &query_embedding, fetch_limit.max(1))
    })
    .await
    .map_err(|error| format!("等待语义精确扫描任务失败: {}", error))?
    .map_err(|error| error.to_string())?;
    let scan_ms = scan_started.elapsed().as_millis() as u64;
    Ok((hits, embedding_ms, scan_ms))
}

#[cfg(test)]
pub(super) async fn rank_paths_for_test(
    db_path: &Path,
    model_dir: &Path,
    query: &str,
) -> Result<Vec<(String, f32)>, String> {
    let query_embedding = embedding::embed_query(model_dir, query, Duration::from_secs(2))
        .await
        .map_err(|error| format!("{}: {}", error.state, error.message))?;
    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let vectors = cached_vectors(&db_path)?;
        let mut ranked = vectors
            .iter()
            .map(|vector| {
                (
                    vector.relative_path.clone(),
                    embedding::cosine_similarity(&query_embedding, &vector.embedding),
                )
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        Ok::<_, anyhow::Error>(ranked)
    })
    .await
    .map_err(|error| format!("等待语义全量排名诊断失败: {}", error))?
    .map_err(|error| error.to_string())
}

fn search_blocking(
    db_path: &Path,
    query_embedding: &[f32],
    fetch_limit: usize,
) -> Result<Vec<SemanticHit>> {
    let vectors = cached_vectors(db_path)?;
    let mut ranked = vectors
        .iter()
        .map(|vector| {
            (
                vector,
                embedding::cosine_similarity(query_embedding, &vector.embedding),
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.relative_path.cmp(&right.0.relative_path))
            .then_with(|| left.0.start_line.cmp(&right.0.start_line))
    });
    ranked.truncate(fetch_limit);

    let connection = open_database(db_path)?;
    let mut hits = Vec::with_capacity(ranked.len());
    for (vector, score) in ranked {
        let excerpt = connection.query_row(
            "SELECT content FROM chunks WHERE rowid = ?1",
            params![vector.chunk_rowid],
            |row| row.get::<_, String>(0),
        )?;
        hits.push(SemanticHit {
            relative_path: vector.relative_path.clone(),
            start_line: vector.start_line,
            end_line: vector.end_line,
            excerpt,
            score,
        });
    }
    Ok(hits)
}

fn cached_vectors(db_path: &Path) -> Result<Arc<Vec<CachedVector>>> {
    let connection = open_database(db_path)?;
    let generation = connection
        .query_row(
            "SELECT value FROM semantic_index_meta WHERE key = 'generation'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default();
    if let Ok(mut cache) = VECTOR_CACHE.lock() {
        if let Some(position) = cache
            .entries
            .iter()
            .position(|entry| entry.db_path == db_path && entry.generation == generation)
        {
            if let Some(entry) = cache.entries.remove(position) {
                let vectors = Arc::clone(&entry.vectors);
                cache.entries.push_front(entry);
                return Ok(vectors);
            }
        }
    }

    let mut statement = connection.prepare(
        "SELECT chunk_key, path, start_line, end_line, chunk_rowid, embedding
         FROM chunk_vectors
         WHERE model_key = ?1 AND dimension = ?2 AND chunk_rowid IS NOT NULL",
    )?;
    let rows = statement.query_map(
        params![model_key(), embedding::MODEL_DIMENSION as i64],
        |row| {
            let blob = row.get::<_, Vec<u8>>(5)?;
            Ok(CachedVector {
                key: row.get(0)?,
                chunk_rowid: row.get(4)?,
                relative_path: row.get(1)?,
                start_line: row.get::<_, i64>(2)? as usize,
                end_line: row.get::<_, i64>(3)? as usize,
                embedding: decode_vector(&blob),
            })
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        let value = row?;
        if value.embedding.len() == embedding::MODEL_DIMENSION {
            values.push(value);
        }
    }
    let bytes = values
        .iter()
        .map(|value| {
            value.key.len()
                + value.relative_path.len()
                + value.embedding.len() * std::mem::size_of::<f32>()
                + 2 * std::mem::size_of::<usize>()
                + std::mem::size_of::<i64>()
        })
        .sum();
    let values = Arc::new(values);
    if let Ok(mut cache) = VECTOR_CACHE.lock() {
        remove_cache_entries(&mut cache, db_path);
        cache.bytes += bytes;
        cache.entries.push_front(CacheEntry {
            db_path: db_path.to_path_buf(),
            generation,
            bytes,
            vectors: Arc::clone(&values),
        });
        while cache.entries.len() > CACHE_PROJECT_LIMIT || cache.bytes > CACHE_BYTE_LIMIT {
            if let Some(removed) = cache.entries.pop_back() {
                cache.bytes = cache.bytes.saturating_sub(removed.bytes);
            } else {
                break;
            }
        }
    }
    Ok(values)
}

pub(super) fn invalidate_cache(db_path: &Path) {
    if let Ok(mut cache) = VECTOR_CACHE.lock() {
        remove_cache_entries(&mut cache, db_path);
    }
}

pub(crate) fn clear_cache() {
    if let Ok(mut cache) = VECTOR_CACHE.lock() {
        cache.entries.clear();
        cache.bytes = 0;
    }
}

fn remove_cache_entries(cache: &mut VectorCache, db_path: &Path) {
    while let Some(position) = cache
        .entries
        .iter()
        .position(|entry| entry.db_path == db_path)
    {
        if let Some(removed) = cache.entries.remove(position) {
            cache.bytes = cache.bytes.saturating_sub(removed.bytes);
        }
    }
}

fn open_database(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("打开 sou 语义索引失败: {}", path.display()))?;
    connection.busy_timeout(Duration::from_millis(250))?;
    ensure_schema(&connection)?;
    Ok(connection)
}

pub(super) fn ensure_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS semantic_index_meta (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS chunk_vectors (
             chunk_key BLOB NOT NULL,
             path TEXT NOT NULL,
             start_line INTEGER NOT NULL,
             end_line INTEGER NOT NULL,
             content_hash BLOB NOT NULL,
             chunk_rowid INTEGER,
             model_key TEXT NOT NULL,
             dimension INTEGER NOT NULL,
             embedding BLOB NOT NULL,
             PRIMARY KEY(chunk_key, model_key)
         );
         CREATE INDEX IF NOT EXISTS idx_chunk_vectors_model
             ON chunk_vectors(model_key);",
    )?;
    let has_chunk_rowid = {
        let mut statement = connection.prepare("PRAGMA table_info(chunk_vectors)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        let mut found = false;
        for row in rows {
            if row? == "chunk_rowid" {
                found = true;
                break;
            }
        }
        found
    };
    if !has_chunk_rowid {
        connection.execute(
            "ALTER TABLE chunk_vectors ADD COLUMN chunk_rowid INTEGER",
            [],
        )?;
    }
    Ok(())
}

fn load_source_chunks(connection: &Connection) -> Result<Vec<SourceChunk>> {
    let mut statement = connection.prepare(
        "SELECT rowid, path, start_line, end_line, content
         FROM chunks ORDER BY path, start_line, end_line",
    )?;
    let rows = statement.query_map([], |row| {
        let rowid = row.get::<_, i64>(0)?;
        let relative_path = row.get::<_, String>(1)?;
        let start_line = row.get::<_, i64>(2)? as usize;
        let end_line = row.get::<_, i64>(3)? as usize;
        let excerpt = row.get::<_, String>(4)?;
        let content_hash = digest(excerpt.as_bytes());
        let key = chunk_key(&relative_path, start_line, end_line, &content_hash);
        Ok(SourceChunk {
            key,
            content_hash,
            rowid,
            relative_path,
            start_line,
            end_line,
            excerpt,
        })
    })?;
    let mut chunks = Vec::new();
    for row in rows {
        chunks.push(row?);
    }
    Ok(chunks)
}

fn load_existing_rows(connection: &Connection, key: &str) -> Result<HashMap<Vec<u8>, Option<i64>>> {
    let mut statement = connection.prepare(
        "SELECT chunk_key, chunk_rowid FROM chunk_vectors
         WHERE model_key = ?1 AND dimension = ?2",
    )?;
    let rows = statement.query_map(params![key, embedding::MODEL_DIMENSION as i64], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    let mut values = HashMap::new();
    for row in rows {
        let (key, rowid) = row?;
        values.insert(key, rowid);
    }
    Ok(values)
}

fn chunk_key(path: &str, start_line: usize, end_line: usize, content_hash: &[u8]) -> Vec<u8> {
    let mut context = ShaContext::new(&SHA256);
    context.update(path.as_bytes());
    context.update(&[0]);
    context.update(&(start_line as u64).to_le_bytes());
    context.update(&(end_line as u64).to_le_bytes());
    context.update(content_hash);
    context.finish().as_ref().to_vec()
}

fn digest(value: &[u8]) -> Vec<u8> {
    let mut context = ShaContext::new(&SHA256);
    context.update(value);
    context.finish().as_ref().to_vec()
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes([value[0], value[1], value[2], value[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::{HashMap, HashSet};
    use std::time::Instant;
    use tempfile::tempdir;

    const GOLDEN_FIXTURE: &str = include_str!("fixtures/semantic_golden.json");

    #[derive(Debug, Deserialize)]
    struct GoldenFixture {
        documents: Vec<GoldenDocument>,
        queries: Vec<GoldenQuery>,
        negative_queries: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct GoldenDocument {
        path: String,
        content: String,
    }

    #[derive(Debug, Deserialize)]
    struct GoldenQuery {
        id: String,
        kind: String,
        query: String,
        relevant_path: String,
    }

    fn load_golden_fixture() -> GoldenFixture {
        serde_json::from_str(GOLDEN_FIXTURE).expect("sou 语义 golden fixture 应为有效 JSON")
    }

    fn create_golden_database(fixture: &GoldenFixture) -> (tempfile::TempDir, PathBuf) {
        let temp = tempdir().expect("应创建语义 golden fixture 目录");
        let db_path = temp.path().join("semantic-golden.sqlite3");
        let connection = Connection::open(&db_path).expect("应创建语义 golden fixture 数据库");
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE chunks USING fts5(
                    path UNINDEXED,
                    start_line UNINDEXED,
                    end_line UNINDEXED,
                    search_text,
                    content UNINDEXED
                 );",
            )
            .expect("应创建 golden fixture 切片表");
        for document in &fixture.documents {
            connection
                .execute(
                    "INSERT INTO chunks(path, start_line, end_line, search_text, content)
                     VALUES (?1, 1, 20, ?2, ?2)",
                    params![document.path, document.content],
                )
                .expect("应写入 golden fixture 文档");
        }
        drop(connection);
        (temp, db_path)
    }

    fn synthetic_gate_vector(seed: usize) -> Vec<f32> {
        let mut state = seed as u64 ^ 0x9e37_79b9_7f4a_7c15;
        (0..embedding::MODEL_DIMENSION)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 40) as f32 / ((1u32 << 24) - 1) as f32) * 2.0 - 1.0
            })
            .collect()
    }

    fn gate_percentile(values: &[u64], percent: usize) -> u64 {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        let index = ((sorted.len() - 1) * percent).div_ceil(100);
        sorted[index.min(sorted.len() - 1)]
    }

    fn sqlite_gate_bytes(path: &Path) -> u64 {
        let wal_path = PathBuf::from(format!("{}-wal", path.display()));
        [path, wal_path.as_path()]
            .iter()
            .filter_map(|value| std::fs::metadata(value).ok())
            .map(|metadata| metadata.len())
            .sum()
    }

    #[test]
    fn golden_fixture_has_layered_stable_contract() {
        let fixture = load_golden_fixture();
        assert!((40..=60).contains(&fixture.queries.len()));
        assert!(fixture.documents.len() >= 10);
        assert!(fixture.negative_queries.len() >= 5);

        let document_paths = fixture
            .documents
            .iter()
            .map(|document| document.path.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(document_paths.len(), fixture.documents.len());

        let query_ids = fixture
            .queries
            .iter()
            .map(|query| query.id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(query_ids.len(), fixture.queries.len());
        for query in &fixture.queries {
            assert!(document_paths.contains(query.relevant_path.as_str()));
        }

        let kinds = fixture
            .queries
            .iter()
            .map(|query| query.kind.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            kinds,
            HashSet::from(["zh_intent", "alias", "exact_identifier"])
        );
    }

    #[test]
    fn chunk_key_changes_with_path_range_or_content() {
        let content = digest(b"same");
        let first = chunk_key("src/a.rs", 1, 80, &content);
        assert_ne!(first, chunk_key("src/b.rs", 1, 80, &content));
        assert_ne!(first, chunk_key("src/a.rs", 2, 80, &content));
        assert_ne!(first, chunk_key("src/a.rs", 1, 80, &digest(b"changed")));
    }

    #[test]
    fn vector_blob_round_trip_preserves_values() {
        let vector = vec![0.25, -0.5, 1.0];
        assert_eq!(decode_vector(&encode_vector(&vector)), vector);
    }

    #[test]
    fn sync_refreshes_rowid_without_reembedding_unchanged_chunk() {
        let temp = tempdir().expect("应创建 rowid 迁移测试目录");
        let db_path = temp.path().join("rowid-refresh.sqlite3");
        let connection = Connection::open(&db_path).expect("应创建 rowid 迁移数据库");
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE chunks USING fts5(
                    path UNINDEXED,
                    start_line UNINDEXED,
                    end_line UNINDEXED,
                    search_text,
                    content UNINDEXED
                 );",
            )
            .expect("应创建 rowid 迁移切片表");
        ensure_schema(&connection).expect("应创建 rowid 迁移 semantic schema");
        let path = "src/rowid.rs";
        let content = "fn stable_chunk() { preserve_embedding(); }";
        connection
            .execute(
                "INSERT INTO chunks(rowid, path, start_line, end_line, search_text, content)
                 VALUES (1, ?1, 1, 20, ?2, ?2)",
                params![path, content],
            )
            .expect("应写入初始 rowid chunk");
        let content_hash = digest(content.as_bytes());
        let identity = chunk_key(path, 1, 20, &content_hash);
        let mut vector = vec![0.0; embedding::MODEL_DIMENSION];
        vector[0] = 1.0;
        connection
            .execute(
                "INSERT INTO chunk_vectors(
                    chunk_key, path, start_line, end_line, content_hash, chunk_rowid,
                    model_key, dimension, embedding
                 ) VALUES (?1, ?2, 1, 20, ?3, 1, ?4, ?5, ?6)",
                params![
                    identity,
                    path,
                    content_hash,
                    model_key(),
                    embedding::MODEL_DIMENSION as i64,
                    encode_vector(&vector)
                ],
            )
            .expect("应写入初始 rowid vector");
        connection
            .execute("DELETE FROM chunks WHERE rowid = 1", [])
            .expect("应删除旧 rowid chunk");
        connection
            .execute(
                "INSERT INTO chunks(rowid, path, start_line, end_line, search_text, content)
                 VALUES (99, ?1, 1, 20, ?2, ?2)",
                params![path, content],
            )
            .expect("应使用新 rowid 重建相同 chunk");
        drop(connection);

        let mut progress = Vec::new();
        sync_vectors(&db_path, Path::new("unused-model"), |indexed, pending| {
            progress.push((indexed, pending));
        })
        .expect("相同 chunk 应只刷新 rowid");
        assert_eq!(progress.first(), Some(&(1, 0)));

        let connection = Connection::open(&db_path).expect("应重新打开 rowid 迁移数据库");
        let refreshed = connection
            .query_row(
                "SELECT chunk_rowid FROM chunk_vectors WHERE chunk_key = ?1 AND model_key = ?2",
                params![identity, model_key()],
                |row| row.get::<_, i64>(0),
            )
            .expect("应读取刷新后的 rowid");
        assert_eq!(refreshed, 99);
        drop(connection);
        invalidate_cache(&db_path);
        let hits = search_blocking(&db_path, &vector, 1).expect("应按新 rowid 读取片段");
        assert_eq!(hits[0].excerpt, content);
    }

    #[test]
    fn semantic_only_threshold_is_separate_from_uiux_threshold() {
        assert!((SEMANTIC_ONLY_THRESHOLD - 0.37).abs() < f32::EPSILON);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "显式使用本机既有 BGE 资产校准小型代码夹具"]
    async fn existing_model_ranks_relevant_code_fixture_above_threshold() {
        let model_dir = embedding::default_model_dir();
        assert!(
            embedding::assets_have_expected_sizes(&model_dir),
            "本机固定 BGE 与 ONNX Runtime 资产应已就绪"
        );
        let temp = tempdir().expect("应创建语义夹具目录");
        let db_path = temp.path().join("semantic-fixture.sqlite3");
        let connection = Connection::open(&db_path).expect("应创建语义夹具数据库");
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE chunks USING fts5(
                    path UNINDEXED,
                    start_line UNINDEXED,
                    end_line UNINDEXED,
                    search_text,
                    content UNINDEXED
                 );",
            )
            .expect("应创建夹具切片表");
        let fixtures = [
            (
                "src/auth/session.rs",
                "fn refresh_expired_access_token(session: &mut Session) { session.rotate_credentials(); }",
            ),
            (
                "src/cache/panel.ts",
                "export function clearPreviewCache() { previewStore.reset(); }",
            ),
            (
                "src/visual/blackhole.rs",
                "fn render_accretion_disk(canvas: &Canvas) { canvas.draw_orbiting_particles(); }",
            ),
            (
                "src/database/migration.rs",
                "fn migrate_schema(connection: &Connection) { connection.apply_pending_migrations(); }",
            ),
        ];
        for (path, content) in fixtures {
            connection
                .execute(
                    "INSERT INTO chunks(path, start_line, end_line, search_text, content)
                     VALUES (?1, 1, 20, ?2, ?2)",
                    params![path, content],
                )
                .expect("应写入代码夹具");
        }
        drop(connection);

        let stats = sync_vectors(&db_path, &model_dir, |_, _| {}).expect("应生成小型代码夹具向量");
        assert_eq!(stats.indexed_chunks, fixtures.len() as u64);
        let mut unchanged_progress = Vec::new();
        sync_vectors(&db_path, &model_dir, |indexed, pending| {
            unchanged_progress.push((indexed, pending));
        })
        .expect("未变化夹具应复用既有向量");
        assert_eq!(
            unchanged_progress.first(),
            Some(&(fixtures.len() as u64, 0))
        );

        let connection = Connection::open(&db_path).expect("应重新打开语义夹具数据库");
        connection
            .execute(
                "UPDATE chunks SET content = ?1, search_text = ?1 WHERE path = ?2",
                params![
                    "export function clearPreviewCache() { previewStore.resetAll(); }",
                    "src/cache/panel.ts"
                ],
            )
            .expect("应更新单个代码夹具");
        drop(connection);
        let mut changed_progress = Vec::new();
        sync_vectors(&db_path, &model_dir, |indexed, pending| {
            changed_progress.push((indexed, pending));
        })
        .expect("变化夹具应增量刷新向量");
        assert_eq!(changed_progress.first(), Some(&(3, 1)));

        let hits = search(
            &db_path,
            &model_dir,
            "会话令牌过期后刷新认证凭据",
            fixtures.len(),
            Duration::from_secs(2),
        )
        .await
        .expect("应完成小型代码语义检索");
        println!(
            "sou semantic fixture scores: {}",
            hits.iter()
                .map(|hit| format!("{}={:.4}", hit.relative_path, hit.score))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(hits[0].relative_path, "src/auth/session.rs");
        assert!(hits[0].score >= SEMANTIC_ONLY_THRESHOLD);
        let unrelated = search(
            &db_path,
            &model_dir,
            "红烧肉需要炖多久以及怎样调整甜度",
            fixtures.len(),
            Duration::from_secs(2),
        )
        .await
        .expect("应完成无关语义查询");
        println!(
            "sou semantic unrelated top: {}={:.4}",
            unrelated[0].relative_path, unrelated[0].score
        );
        assert!(unrelated[0].score < SEMANTIC_ONLY_THRESHOLD);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "显式使用本机既有 BGE 资产执行 40 条分层质量门禁"]
    async fn existing_model_meets_layered_golden_quality_gate() {
        let model_dir = embedding::default_model_dir();
        assert!(
            embedding::assets_have_expected_sizes(&model_dir),
            "本机固定 BGE 与 ONNX Runtime 资产应已就绪"
        );
        let fixture = load_golden_fixture();
        let (_temp, db_path) = create_golden_database(&fixture);
        let stats =
            sync_vectors(&db_path, &model_dir, |_, _| {}).expect("应生成分层 golden fixture 向量");
        assert_eq!(stats.indexed_chunks, fixture.documents.len() as u64);

        let mut top_one = 0usize;
        let mut top_five = 0usize;
        let mut kind_totals = HashMap::<String, usize>::new();
        let mut kind_top_five = HashMap::<String, usize>::new();
        for query in &fixture.queries {
            let hits = search(
                &db_path,
                &model_dir,
                &query.query,
                fixture.documents.len(),
                Duration::from_secs(2),
            )
            .await
            .expect("应完成 golden 查询");
            *kind_totals.entry(query.kind.clone()).or_default() += 1;
            if hits.first().map(|hit| hit.relative_path.as_str())
                == Some(query.relevant_path.as_str())
            {
                top_one += 1;
            }
            if hits
                .iter()
                .take(5)
                .any(|hit| hit.relative_path == query.relevant_path)
            {
                top_five += 1;
                *kind_top_five.entry(query.kind.clone()).or_default() += 1;
            }
        }

        let mut rejected_negative = 0usize;
        for query in &fixture.negative_queries {
            let hits = search(
                &db_path,
                &model_dir,
                query,
                fixture.documents.len(),
                Duration::from_secs(2),
            )
            .await
            .expect("应完成 golden 无关查询");
            if hits
                .first()
                .map(|hit| hit.score < SEMANTIC_ONLY_THRESHOLD)
                .unwrap_or(true)
            {
                rejected_negative += 1;
            }
        }

        let recall_at_one = top_one as f32 / fixture.queries.len() as f32;
        let recall_at_five = top_five as f32 / fixture.queries.len() as f32;
        let negative_rejection = rejected_negative as f32 / fixture.negative_queries.len() as f32;
        println!(
            "sou semantic golden: recall@1={recall_at_one:.3}, recall@5={recall_at_five:.3}, negative_rejection={negative_rejection:.3}, by_kind={kind_top_five:?}/{kind_totals:?}"
        );

        assert!(recall_at_five >= 0.80);
        assert!(negative_rejection >= 0.80);
        let intent_total = kind_totals.get("zh_intent").copied().unwrap_or(0);
        let intent_hits = kind_top_five.get("zh_intent").copied().unwrap_or(0);
        assert_eq!(intent_hits, intent_total);

        // 精确标识符由词法通道兜底；这里仅防止语义通道覆盖率显著退化。
        let exact_total = kind_totals.get("exact_identifier").copied().unwrap_or(0);
        let exact_hits = kind_top_five.get("exact_identifier").copied().unwrap_or(0);
        assert!(exact_total > 0);
        assert!(exact_hits as f32 / exact_total as f32 >= 0.75);
    }

    #[test]
    #[ignore = "由 phase2 门禁脚本在 release 测试二进制中执行 20k exact scan"]
    fn phase2_gate_20k_exact_scan_from_env() {
        let chunk_count = std::env::var("SANSHU_SOU_20K_CHUNKS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(20_000);
        let max_p95_ms = std::env::var("SANSHU_SOU_20K_MAX_P95_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120);
        assert!(chunk_count >= 20_000);

        let temp = tempdir().expect("应创建 20k exact scan 门禁目录");
        let db_path = temp.path().join("semantic-20k.sqlite3");
        let mut connection = Connection::open(&db_path).expect("应创建 20k 门禁数据库");
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 CREATE VIRTUAL TABLE chunks USING fts5(
                    path UNINDEXED,
                    start_line UNINDEXED,
                    end_line UNINDEXED,
                    search_text,
                    content UNINDEXED
                 );",
            )
            .expect("应创建 20k 门禁切片表");
        ensure_schema(&connection).expect("应创建 20k semantic schema");

        let query_vector = (0..embedding::MODEL_DIMENSION)
            .map(|index| if index % 17 == 0 { 1.0 } else { 0.0 })
            .collect::<Vec<_>>();
        let target_index = chunk_count / 2;
        let target_path = "src/generated/semantic_target.rs";
        let transaction = connection.transaction().expect("应开启 20k 写事务");
        {
            let mut chunk_statement = transaction
                .prepare(
                    "INSERT INTO chunks(path, start_line, end_line, search_text, content)
                     VALUES (?1, 1, 20, ?2, ?2)",
                )
                .expect("应准备 20k chunk 写入");
            let mut vector_statement = transaction
                .prepare(
                    "INSERT INTO chunk_vectors(
                            chunk_key, path, start_line, end_line, content_hash, chunk_rowid,
                            model_key, dimension, embedding
                         ) VALUES (?1, ?2, 1, 20, ?3, ?4, ?5, ?6, ?7)",
                )
                .expect("应准备 20k vector 写入");
            let key = model_key();
            for index in 0..chunk_count {
                let path = if index == target_index {
                    target_path.to_string()
                } else {
                    format!("src/generated/module_{index:05}.rs")
                };
                let content = if index == target_index {
                    "fn locate_semantic_target() { return_exact_match(); }".to_string()
                } else {
                    format!("fn generated_module_{index:05}() {{ process_fixture(); }}")
                };
                let content_hash = digest(content.as_bytes());
                let identity = chunk_key(&path, 1, 20, &content_hash);
                let vector = if index == target_index {
                    query_vector.clone()
                } else {
                    synthetic_gate_vector(index)
                };
                chunk_statement
                    .execute(params![path, content])
                    .expect("应写入 20k chunk");
                let chunk_rowid = transaction.last_insert_rowid();
                vector_statement
                    .execute(params![
                        identity,
                        path,
                        content_hash,
                        chunk_rowid,
                        key,
                        embedding::MODEL_DIMENSION as i64,
                        encode_vector(&vector)
                    ])
                    .expect("应写入 20k vector");
                if (index + 1) % 5_000 == 0 {
                    println!(
                        "SOU_PHASE2_GATE_PROGRESS synthetic_indexed={}/{}",
                        index + 1,
                        chunk_count
                    );
                }
            }
        }
        transaction.commit().expect("应提交 20k 写事务");
        connection
            .execute(
                "INSERT OR REPLACE INTO semantic_index_meta(key, value) VALUES ('generation', 'phase2-20k')",
                [],
            )
            .expect("应写入 20k 缓存代次");
        connection
            .execute(
                "INSERT OR REPLACE INTO semantic_index_meta(key, value) VALUES ('model_key', ?1)",
                params![model_key()],
            )
            .expect("应写入 20k 模型身份");
        drop(connection);
        invalidate_cache(&db_path);

        let cold_started = Instant::now();
        let cold_hits =
            search_blocking(&db_path, &query_vector, 20).expect("20k exact scan 冷缓存查询应成功");
        let cold_ms = cold_started.elapsed().as_millis() as u64;
        assert_eq!(cold_hits[0].relative_path, target_path);

        let mut durations = Vec::new();
        for _ in 0..40 {
            let started = Instant::now();
            let hits = search_blocking(&db_path, &query_vector, 20)
                .expect("20k exact scan warm 查询应成功");
            assert_eq!(hits[0].relative_path, target_path);
            durations.push(started.elapsed().as_millis() as u64);
        }
        let p50_ms = gate_percentile(&durations, 50);
        let p95_ms = gate_percentile(&durations, 95);
        let result = serde_json::json!({
            "type": "synthetic_20k",
            "chunks": chunk_count,
            "dimension": embedding::MODEL_DIMENSION,
            "cold_cache_ms": cold_ms,
            "warm_query_count": durations.len(),
            "warm_p50_ms": p50_ms,
            "warm_p95_ms": p95_ms,
            "max_p95_ms": max_p95_ms,
            "sqlite_bytes": sqlite_gate_bytes(&db_path),
            "estimated_vector_bytes": chunk_count * embedding::MODEL_DIMENSION * std::mem::size_of::<f32>(),
            "top_one_path": target_path,
        });
        println!("SOU_PHASE2_20K_RESULT={result}");
        assert!(p95_ms <= max_p95_ms);
    }

    #[test]
    #[ignore = "显式使用本机既有 BGE 资产比较语义索引吞吐变体"]
    fn benchmark_sync_throughput_variants() {
        let chunk_count = std::env::var("SANSHU_SOU_BENCH_CHUNKS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(512)
            .max(64);
        let model_dir = embedding::default_model_dir();
        assert!(
            embedding::assets_have_expected_sizes(&model_dir),
            "本机固定 BGE 与 ONNX Runtime 资产应已就绪"
        );

        // 中文说明：先完成一次短 batch 预热，把模型加载和首轮算子初始化从 A/B 数据中剥离。
        embedding::embed_documents_blocking(
            &model_dir,
            vec!["语义索引吞吐基准预热：代码上下文".to_string()],
            Duration::from_secs(120),
        )
        .expect("BGE 预热应成功");

        let variants = [
            ("batch32_per_batch_txn", 32usize, true),
            ("batch64_per_batch_txn", 64usize, true),
            ("batch128_per_batch_txn", 128usize, true),
            ("batch32_single_txn", 32usize, false),
        ];
        let mut results = Vec::with_capacity(variants.len());
        for (name, batch_size, commit_each_batch) in variants {
            let temp = tempdir().expect("应创建吞吐基准临时目录");
            let db_path = temp.path().join("semantic-throughput.sqlite3");
            let connection = Connection::open(&db_path).expect("应创建吞吐基准数据库");
            connection
                .execute_batch(
                    "CREATE VIRTUAL TABLE chunks USING fts5(
                        path UNINDEXED,
                        start_line UNINDEXED,
                        end_line UNINDEXED,
                        search_text,
                        content UNINDEXED
                     );",
                )
                .expect("应创建吞吐基准切片表");
            {
                let transaction = connection
                    .unchecked_transaction()
                    .expect("应开启吞吐基准夹具事务");
                for index in 0..chunk_count {
                    let path = format!("src/bench/module_{index:05}.rs");
                    let content = format!(
                        "fn generated_module_{index:05}() {{ process semantic indexing fixture; token_group {}; }}",
                        index % 17
                    );
                    transaction
                        .execute(
                            "INSERT INTO chunks(path, start_line, end_line, search_text, content)
                             VALUES (?1, 1, 20, ?2, ?2)",
                            params![path, content],
                        )
                        .expect("应写入吞吐基准 chunk");
                }
                transaction.commit().expect("应提交吞吐基准夹具事务");
            }
            drop(connection);

            let started = Instant::now();
            let stats = sync_vectors_with_tuning(
                &db_path,
                &model_dir,
                |_, _| {},
                SemanticSyncTuning {
                    embedding_batch_size: batch_size,
                    commit_each_batch,
                    dynamic_resources: false,
                },
            )
            .expect("吞吐基准同步应成功");
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let chunks_per_second = if elapsed_ms == 0 {
                0.0
            } else {
                stats.indexed_chunks as f64 * 1000.0 / elapsed_ms as f64
            };
            results.push(serde_json::json!({
                "name": name,
                "batch_size": batch_size,
                "commit_each_batch": commit_each_batch,
                "chunks": stats.indexed_chunks,
                "elapsed_ms": elapsed_ms,
                "chunks_per_second": chunks_per_second,
            }));
        }
        let snapshot = embedding::snapshot(&model_dir);
        let result = serde_json::json!({
            "type": "semantic_sync_throughput",
            "provider": snapshot.execution_provider,
            "requested_provider": snapshot.requested_provider,
            "chunks": chunk_count,
            "results": results,
        });
        println!("SOU_SEMANTIC_THROUGHPUT_RESULT={result}");
    }
}
