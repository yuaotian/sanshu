//! sou Local 后端的 BGE 向量持久化、精确扫描与有界内存缓存。

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use ring::digest::{Context as ShaContext, SHA256};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::mcp::embedding;

pub(super) const FUSION_NAME: &str = "weighted_rrf_0.65_0.35_k60";
pub(super) const SEMANTIC_ONLY_THRESHOLD: f32 = 0.37;
const CACHE_PROJECT_LIMIT: usize = 2;
const CACHE_BYTE_LIMIT: usize = 256 * 1024 * 1024;
const EMBEDDING_BATCH_SIZE: usize = 32;

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
    relative_path: String,
    start_line: usize,
    end_line: usize,
    excerpt: String,
}

struct CachedVector {
    key: Vec<u8>,
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
    let connection = open_database(db_path)?;
    let total = connection.query_row("SELECT COUNT(*) FROM chunks", [], |row| {
        row.get::<_, u64>(0)
    })?;
    let indexed = connection.query_row(
        "SELECT COUNT(*) FROM chunk_vectors WHERE model_key = ?1 AND dimension = ?2",
        params![model_key(), embedding::MODEL_DIMENSION as i64],
        |row| row.get::<_, u64>(0),
    )?;
    Ok(SemanticIndexStats {
        indexed_chunks: indexed.min(total),
        pending_chunks: total.saturating_sub(indexed.min(total)),
    })
}

pub(super) fn sync_vectors<F>(
    db_path: &Path,
    model_dir: &Path,
    mut on_progress: F,
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
    let existing = load_existing_keys(&connection, &key)?;

    {
        let transaction = connection.transaction()?;
        for stale in existing.difference(&current_keys) {
            transaction.execute(
                "DELETE FROM chunk_vectors WHERE chunk_key = ?1 AND model_key = ?2",
                params![stale, key],
            )?;
        }
        transaction.execute(
            "DELETE FROM chunk_vectors WHERE model_key = ?1 AND dimension != ?2",
            params![key, embedding::MODEL_DIMENSION as i64],
        )?;
        transaction.commit()?;
    }

    let mut pending = chunks
        .iter()
        .filter(|chunk| !existing.contains(&chunk.key))
        .cloned()
        .collect::<Vec<_>>();
    let total = chunks.len() as u64;
    let mut indexed = total.saturating_sub(pending.len() as u64);
    on_progress(indexed, pending.len() as u64);

    for batch in pending.chunks_mut(EMBEDDING_BATCH_SIZE) {
        let documents = batch
            .iter()
            .map(|chunk| format!("{}\n{}", chunk.relative_path, chunk.excerpt))
            .collect::<Vec<_>>();
        let embeddings =
            embedding::embed_documents_blocking(model_dir, documents, Duration::from_secs(120))
                .map_err(|error| anyhow::anyhow!("{}: {}", error.state, error.message))?;
        let transaction = connection.transaction()?;
        for (chunk, vector) in batch.iter().zip(embeddings) {
            transaction.execute(
                "INSERT OR REPLACE INTO chunk_vectors(
                    chunk_key, path, start_line, end_line, content_hash,
                    model_key, dimension, embedding
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    chunk.key,
                    chunk.relative_path,
                    chunk.start_line as i64,
                    chunk.end_line as i64,
                    chunk.content_hash,
                    key,
                    embedding::MODEL_DIMENSION as i64,
                    encode_vector(&vector),
                ],
            )?;
        }
        transaction.commit()?;
        indexed += batch.len() as u64;
        on_progress(indexed, total.saturating_sub(indexed));
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
            "SELECT content FROM chunks
             WHERE path = ?1 AND start_line = ?2 AND end_line = ?3
             LIMIT 1",
            params![
                vector.relative_path,
                vector.start_line as i64,
                vector.end_line as i64
            ],
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
        "SELECT chunk_key, path, start_line, end_line, embedding
         FROM chunk_vectors
         WHERE model_key = ?1 AND dimension = ?2",
    )?;
    let rows = statement.query_map(
        params![model_key(), embedding::MODEL_DIMENSION as i64],
        |row| {
            let blob = row.get::<_, Vec<u8>>(4)?;
            Ok(CachedVector {
                key: row.get(0)?,
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
             model_key TEXT NOT NULL,
             dimension INTEGER NOT NULL,
             embedding BLOB NOT NULL,
             PRIMARY KEY(chunk_key, model_key)
         );
         CREATE INDEX IF NOT EXISTS idx_chunk_vectors_model
             ON chunk_vectors(model_key);",
    )?;
    Ok(())
}

fn load_source_chunks(connection: &Connection) -> Result<Vec<SourceChunk>> {
    let mut statement = connection.prepare(
        "SELECT path, start_line, end_line, content
         FROM chunks ORDER BY path, start_line, end_line",
    )?;
    let rows = statement.query_map([], |row| {
        let relative_path = row.get::<_, String>(0)?;
        let start_line = row.get::<_, i64>(1)? as usize;
        let end_line = row.get::<_, i64>(2)? as usize;
        let excerpt = row.get::<_, String>(3)?;
        let content_hash = digest(excerpt.as_bytes());
        let key = chunk_key(&relative_path, start_line, end_line, &content_hash);
        Ok(SourceChunk {
            key,
            content_hash,
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

fn load_existing_keys(connection: &Connection, key: &str) -> Result<HashSet<Vec<u8>>> {
    let mut statement = connection
        .prepare("SELECT chunk_key FROM chunk_vectors WHERE model_key = ?1 AND dimension = ?2")?;
    let rows = statement.query_map(params![key, embedding::MODEL_DIMENSION as i64], |row| {
        row.get::<_, Vec<u8>>(0)
    })?;
    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row?);
    }
    Ok(keys)
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
}
