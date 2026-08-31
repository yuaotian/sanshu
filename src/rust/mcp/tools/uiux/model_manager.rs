//! UIUX 本地 BGE 模型的下载、校验、索引缓存与进程内推理生命周期。

use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use reqwest::header::RANGE;
use ring::digest::{Context as ShaContext, SHA256};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::{load_standalone_config, AppState, ProxyConfig};
use crate::log_important;
use crate::network::download_verified_with_strategy_with_progress_and_cancel;
use crate::network::proxy::{ProxyDetector, ProxyInfo, ProxyType};

use super::structured_search;

pub const MODEL_NAME: &str = "Xenova/bge-small-zh-v1.5";
pub const MODEL_REVISION: &str = "75c43b069aac4d136ba6bc1122f995fedcfd2781";
pub const MODEL_DIMENSION: usize = 512;
pub const QUERY_PREFIX: &str = "为这个句子生成表示以用于检索相关文章：";
pub const ORT_VERSION: &str = "1.28.0";

const MODEL_TOTAL_BYTES: u64 = 95_292_210;
const ORT_ARCHIVE_BYTES: u64 = 78_796_801;
const DOWNLOAD_TOTAL_BYTES: u64 = MODEL_TOTAL_BYTES + ORT_ARCHIVE_BYTES;
const STATUS_FILE_NAME: &str = "uiux_model_status.json";
const LOCK_FILE_NAME: &str = ".uiux-model.lock";
const INDEX_LOCK_FILE_NAME: &str = ".uiux-index.lock";
const ORT_LOCK_FILE_NAME: &str = ".onnxruntime-download.lock";
const EMBEDDING_CACHE_FILE: &str = "uiux-pro-max-v2.15.0-bge.f32";
const CACHE_MAGIC: &[u8; 8] = b"UIUXBGE1";
const DOWNLOAD_TIMEOUT_SECS: u64 = 1_800;
const CONNECT_TIMEOUT_SECS: u64 = 8;
const ORT_ARCHIVE_FILE_NAME: &str = "onnxruntime-win-x64-1.28.0.zip";
const ORT_ARCHIVE_URL: &str =
    "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-win-x64-1.28.0.zip";
const ORT_ARCHIVE_SHA256: &str = "abef733dacbe2f571547a7150b479b5cb9cc0df22f96c24983a42cadb1b4f8bc";
const ORT_DLL_ARCHIVE_PATH: &str = "onnxruntime-win-x64-1.28.0/lib/onnxruntime.dll";
const ORT_DLL_FILE_NAME: &str = "onnxruntime.dll";
const ORT_DLL_BYTES: u64 = 15_809_848;
const ORT_DLL_SHA256: &str = "18370c375f07357fa5874344a9d9ac17e6b6fe1eb18b1dd209d79483b4470257";
const ORT_LICENSE_ARCHIVE_PATH: &str = "onnxruntime-win-x64-1.28.0/LICENSE";
const ORT_LICENSE_FILE_NAME: &str = "LICENSE.onnxruntime";
const ORT_LICENSE_BYTES: u64 = 1_094;
const ORT_LICENSE_SHA256: &str = "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674";

#[derive(Clone, Copy)]
struct ModelFileSpec {
    relative_path: &'static str,
    size: u64,
    sha256: &'static str,
}

const MODEL_FILES: &[ModelFileSpec] = &[
    ModelFileSpec {
        relative_path: "onnx/model.onnx",
        size: 94_851_877,
        sha256: "69a0b846f4f116b5e6aabf9546ea6754d02264f3211a13a1bd69b31b8040749a",
    },
    ModelFileSpec {
        relative_path: "config.json",
        size: 716,
        sha256: "d4193ead3a810fd694fa8a31d7fc72fbaebc0668b603e398734bf2f6538ff42f",
    },
    ModelFileSpec {
        relative_path: "special_tokens_map.json",
        size: 125,
        sha256: "b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3",
    },
    ModelFileSpec {
        relative_path: "tokenizer.json",
        size: 439_125,
        sha256: "48cea5d44424912a6fd1ea647bf4fe50b55ab8b1e5879c3275f80e339e8fae26",
    },
    ModelFileSpec {
        relative_path: "tokenizer_config.json",
        size: 367,
        sha256: "e6f3b96db926a37d4039995fbf5ad17de158dfb8f6343d607e4dbaad18d75f5a",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiuxModelStatus {
    pub phase: String,
    pub model_name: String,
    pub revision: String,
    pub model_dir: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub model_downloaded_bytes: u64,
    pub model_total_bytes: u64,
    pub completed_files: usize,
    pub total_files: usize,
    pub runtime_version: String,
    pub runtime_ready: bool,
    pub runtime_dir: String,
    pub runtime_downloaded_bytes: u64,
    pub runtime_total_bytes: u64,
    pub indexed_documents: usize,
    pub total_documents: usize,
    pub progress_percent: f64,
    #[serde(default)]
    pub index_progress_percent: f64,
    pub route: Option<String>,
    pub message: String,
    pub error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiuxConfig {
    pub knowledge_backend: String,
    pub semantic_enabled: bool,
    pub model_dir: Option<String>,
    pub effective_model_dir: String,
}

#[derive(Debug, Clone)]
pub struct SemanticMatch {
    pub document_index: usize,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct SemanticRanking {
    pub matches: Vec<SemanticMatch>,
    pub top_score: f32,
}

#[derive(Debug, Clone)]
pub struct SemanticSettings {
    pub enabled: bool,
    pub model_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SemanticUnavailable {
    pub state: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimePhase {
    Empty,
    Loading,
    Ready,
    Error,
}

struct RuntimeSlot {
    directory: Option<PathBuf>,
    phase: RuntimePhase,
    model: Option<TextEmbedding>,
    embeddings: Vec<Vec<f32>>,
    error: Option<String>,
}

impl Default for RuntimeSlot {
    fn default() -> Self {
        Self {
            directory: None,
            phase: RuntimePhase::Empty,
            model: None,
            embeddings: Vec::new(),
            error: None,
        }
    }
}

static RUNTIME: Lazy<Mutex<RuntimeSlot>> = Lazy::new(|| Mutex::new(RuntimeSlot::default()));
static DOWNLOAD_RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL_DOWNLOAD: AtomicBool = AtomicBool::new(false);

pub fn semantic_settings() -> SemanticSettings {
    let config = load_standalone_config().ok();
    let mcp = config.as_ref().map(|value| &value.mcp_config);
    SemanticSettings {
        enabled: mcp
            .and_then(|value| value.uiux_semantic_enabled)
            .unwrap_or(true),
        model_dir: effective_model_dir(mcp.and_then(|value| value.uiux_model_dir.as_deref())),
    }
}

pub fn effective_model_dir(configured: Option<&str>) -> PathBuf {
    if let Some(path) = configured.map(str::trim).filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("sanshu")
        .join("models")
        .join("bge-small-zh-v1.5")
}

fn effective_runtime_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("sanshu")
        .join("runtimes")
        .join(format!("onnxruntime-{}", ORT_VERSION))
}

#[tauri::command]
pub fn get_uiux_config(state: tauri::State<'_, AppState>) -> Result<UiuxConfig, String> {
    let config = state
        .config
        .lock()
        .map_err(|error| format!("获取 UIUX 配置失败: {}", error))?;
    let mcp = &config.mcp_config;
    let model_dir = mcp.uiux_model_dir.clone();
    Ok(UiuxConfig {
        knowledge_backend: mcp
            .uiux_knowledge_backend
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        semantic_enabled: mcp.uiux_semantic_enabled.unwrap_or(true),
        effective_model_dir: effective_model_dir(model_dir.as_deref())
            .to_string_lossy()
            .to_string(),
        model_dir,
    })
}

#[tauri::command]
pub async fn set_uiux_config(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    config: UiuxConfig,
) -> Result<(), String> {
    let backend = config
        .knowledge_backend
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    if !matches!(backend.as_str(), "auto" | "fast_context" | "local") {
        return Err(format!("未知 UIUX 检索后端: {}", config.knowledge_backend));
    }
    let model_dir = config
        .model_dir
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let new_directory = effective_model_dir(model_dir.as_deref());

    let previous = {
        let mut app_config = state
            .config
            .lock()
            .map_err(|error| format!("锁定 UIUX 配置失败: {}", error))?;
        let previous = (
            app_config.mcp_config.uiux_knowledge_backend.clone(),
            app_config.mcp_config.uiux_semantic_enabled,
            app_config.mcp_config.uiux_model_dir.clone(),
        );
        app_config.mcp_config.uiux_knowledge_backend = Some(backend.clone());
        app_config.mcp_config.uiux_semantic_enabled = Some(config.semantic_enabled);
        app_config.mcp_config.uiux_model_dir = model_dir.clone();
        previous
    };
    if let Err(error) = crate::config::save_config(&state, &app_handle).await {
        if let Ok(mut app_config) = state.config.lock() {
            let still_owns_values = app_config.mcp_config.uiux_knowledge_backend.as_deref()
                == Some(backend.as_str())
                && app_config.mcp_config.uiux_semantic_enabled == Some(config.semantic_enabled)
                && app_config.mcp_config.uiux_model_dir == model_dir;
            if still_owns_values {
                app_config.mcp_config.uiux_knowledge_backend = previous.0;
                app_config.mcp_config.uiux_semantic_enabled = previous.1;
                app_config.mcp_config.uiux_model_dir = previous.2;
            }
        }
        return Err(format!("保存 UIUX 配置失败: {}", error));
    }

    reset_runtime_if_directory_changed(&new_directory);
    Ok(())
}

#[tauri::command]
pub async fn select_uiux_model_directory(
    app_handle: tauri::AppHandle,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let mut builder = app_handle.dialog().file();
    if let Some(path) = default_path {
        let path = PathBuf::from(path);
        if path.exists() {
            builder = builder.set_directory(path);
        }
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    builder.pick_folder(move |path| {
        let _ = sender.send(path);
    });
    receiver
        .await
        .map(|path| path.map(|value| value.to_string()))
        .map_err(|_| "模型目录选择已取消".to_string())
}

#[tauri::command]
pub fn get_uiux_model_status(state: tauri::State<'_, AppState>) -> Result<UiuxModelStatus, String> {
    let directory = {
        let config = state
            .config
            .lock()
            .map_err(|error| format!("获取 UIUX 配置失败: {}", error))?;
        effective_model_dir(config.mcp_config.uiux_model_dir.as_deref())
    };
    if assets_have_expected_sizes(&directory) {
        ensure_runtime_started(&directory);
    }
    Ok(current_status(&directory))
}

#[tauri::command]
pub async fn start_uiux_model_download(
    state: tauri::State<'_, AppState>,
) -> Result<UiuxModelStatus, String> {
    if DOWNLOAD_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("UIUX 模型下载任务已在运行".to_string());
    }
    CANCEL_DOWNLOAD.store(false, Ordering::SeqCst);

    let prepared = (|| {
        let config = state
            .config
            .lock()
            .map_err(|error| format!("获取 UIUX 配置失败: {}", error))?;
        let directory = effective_model_dir(config.mcp_config.uiux_model_dir.as_deref());
        let proxy_config = config.proxy_config.clone();
        drop(config);
        let initial = status_for(&directory, "downloading", "准备下载模型文件");
        write_status(&initial)?;
        Ok::<_, String>((directory, proxy_config, initial))
    })();
    let (directory, proxy_config, initial) = match prepared {
        Ok(value) => value,
        Err(error) => {
            DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
            return Err(error);
        }
    };

    let task_directory = directory.clone();
    tauri::async_runtime::spawn(async move {
        let result = download_model(&task_directory, &proxy_config).await;
        DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                ensure_runtime_started(&task_directory);
                let _ = write_status(&current_status(&task_directory));
            }
            Err(error) => {
                let cancelled = CANCEL_DOWNLOAD.load(Ordering::SeqCst);
                let phase = if cancelled { "missing" } else { "error" };
                let message = if cancelled {
                    "模型下载已取消"
                } else {
                    "模型下载未完成"
                };
                let mut status = status_for(&task_directory, phase, message);
                if !cancelled {
                    status.error = Some(error.clone());
                }
                let _ = write_status(&status);
                log_important!(warn, "[uiux_model] 模型下载失败: {}", error);
            }
        }
    });
    Ok(initial)
}

#[tauri::command]
pub fn cancel_uiux_model_download() -> Result<(), String> {
    CANCEL_DOWNLOAD.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn remove_uiux_model(state: tauri::State<'_, AppState>) -> Result<UiuxModelStatus, String> {
    if DOWNLOAD_RUNNING.load(Ordering::SeqCst) {
        return Err("请先取消正在运行的 UIUX 模型下载".to_string());
    }
    let directory = {
        let config = state
            .config
            .lock()
            .map_err(|error| format!("获取 UIUX 配置失败: {}", error))?;
        effective_model_dir(config.mcp_config.uiux_model_dir.as_deref())
    };
    fs::create_dir_all(&directory)
        .map_err(|error| format!("创建模型目录失败 {}: {}", directory.display(), error))?;
    let _download_lease = try_acquire_lease(
        &directory.join(LOCK_FILE_NAME),
        "另一个 Sanshu 进程正在下载 UIUX 模型",
    )?;
    let _index_lease = try_acquire_lease(
        &directory.join(INDEX_LOCK_FILE_NAME),
        "另一个 Sanshu 进程正在加载 UIUX 模型或建立语义索引",
    )?;
    remove_known_model_files(&directory)?;
    reset_runtime();
    let status = status_for(
        &directory,
        "missing",
        "BGE 模型已移除；共享 ORT 运行时保留，auto 将继续使用 BM25",
    );
    write_status(&status)?;
    Ok(status)
}

pub async fn rank_documents(
    query: &str,
    directory: &Path,
    wait_budget: Duration,
) -> Result<SemanticRanking, SemanticUnavailable> {
    ensure_runtime_started(directory);
    let deadline = Instant::now() + wait_budget;
    loop {
        let snapshot = runtime_snapshot(directory);
        match snapshot.0 {
            RuntimePhase::Ready => break,
            RuntimePhase::Error => {
                return Err(SemanticUnavailable {
                    state: "error".to_string(),
                    message: snapshot
                        .1
                        .unwrap_or_else(|| "BGE 运行时初始化失败".to_string()),
                });
            }
            RuntimePhase::Empty => {
                return Err(SemanticUnavailable {
                    state: "missing".to_string(),
                    message: "BGE 模型文件尚未下载完整".to_string(),
                });
            }
            RuntimePhase::Loading if Instant::now() >= deadline => {
                return Err(SemanticUnavailable {
                    state: "loading".to_string(),
                    message: "BGE 正在加载或建立语义索引，本次使用 BM25".to_string(),
                });
            }
            RuntimePhase::Loading => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }

    let query = format!("{}{}", QUERY_PREFIX, query.trim());
    tokio::task::spawn_blocking(move || rank_documents_blocking(&query))
        .await
        .map_err(|error| SemanticUnavailable {
            state: "error".to_string(),
            message: format!("等待 BGE 推理任务失败: {}", error),
        })?
}

fn rank_documents_blocking(query: &str) -> Result<SemanticRanking, SemanticUnavailable> {
    let mut runtime = RUNTIME.lock().map_err(|error| SemanticUnavailable {
        state: "error".to_string(),
        message: format!("锁定 BGE 运行时失败: {}", error),
    })?;
    let RuntimeSlot {
        model,
        embeddings,
        phase,
        ..
    } = &mut *runtime;
    if *phase != RuntimePhase::Ready {
        return Err(SemanticUnavailable {
            state: "loading".to_string(),
            message: "BGE 运行时尚未就绪".to_string(),
        });
    }
    let model = model.as_mut().ok_or_else(|| SemanticUnavailable {
        state: "error".to_string(),
        message: "BGE 运行时缺少模型实例".to_string(),
    })?;
    let query_embedding = model
        .embed(vec![query], Some(1))
        .map_err(|error| SemanticUnavailable {
            state: "error".to_string(),
            message: format!("BGE 查询向量生成失败: {}", error),
        })?
        .into_iter()
        .next()
        .ok_or_else(|| SemanticUnavailable {
            state: "error".to_string(),
            message: "BGE 未返回查询向量".to_string(),
        })?;

    let mut matches = embeddings
        .iter()
        .enumerate()
        .map(|(document_index, embedding)| SemanticMatch {
            document_index,
            score: cosine_similarity(&query_embedding, embedding),
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.document_index.cmp(&right.document_index))
    });
    let top_score = matches.first().map(|value| value.score).unwrap_or_default();
    Ok(SemanticRanking { matches, top_score })
}

fn ensure_runtime_started(directory: &Path) {
    let directory = directory.to_path_buf();
    let mut runtime = match RUNTIME.lock() {
        Ok(value) => value,
        Err(error) => {
            log_important!(warn, "[uiux_model] 锁定运行时失败: {}", error);
            return;
        }
    };
    if runtime.directory.as_deref() == Some(directory.as_path())
        && matches!(runtime.phase, RuntimePhase::Loading | RuntimePhase::Ready)
    {
        return;
    }
    if !assets_have_expected_sizes(&directory) {
        runtime.directory = Some(directory);
        runtime.phase = RuntimePhase::Empty;
        runtime.model = None;
        runtime.embeddings.clear();
        return;
    }
    runtime.directory = Some(directory.clone());
    runtime.phase = RuntimePhase::Loading;
    runtime.model = None;
    runtime.embeddings.clear();
    runtime.error = None;
    drop(runtime);

    let mut status = status_for(&directory, "loading", "正在校验并加载 BGE 模型");
    mark_download_complete(&mut status);
    status.progress_percent = 100.0;
    let _ = write_status(&status);
    std::thread::spawn(move || match load_runtime(&directory) {
        Ok((model, embeddings)) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path()) {
                    runtime.model = Some(model);
                    runtime.embeddings = embeddings;
                    runtime.phase = RuntimePhase::Ready;
                    runtime.error = None;
                }
            }
            let mut status = status_for(&directory, "ready", "BGE 模型与 UIUX 语义索引已就绪");
            mark_download_complete(&mut status);
            status.indexed_documents = structured_search::semantic_documents().len();
            status.index_progress_percent = 100.0;
            let _ = write_status(&status);
        }
        Err(error) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path()) {
                    runtime.phase = RuntimePhase::Error;
                    runtime.error = Some(error.clone());
                }
            }
            let mut status = status_for(&directory, "error", "BGE 模型初始化失败");
            status.error = Some(error.clone());
            let _ = write_status(&status);
            log_important!(warn, "[uiux_model] 初始化失败: {}", error);
        }
    });
}

fn load_runtime(directory: &Path) -> Result<(TextEmbedding, Vec<Vec<f32>>), String> {
    let corpus_hash = corpus_hash();
    let cache_path = directory.join(EMBEDDING_CACHE_FILE);
    let index_lock_path = directory.join(INDEX_LOCK_FILE_NAME);
    // GUI 与 MCP 是独立进程；模型加载、缓存生成和删除必须共享同一把文件锁。
    let _index_lease = wait_for_index_lease(&index_lock_path, &cache_path, &corpus_hash)?;
    verify_model_files(directory)?;
    verify_runtime_assets()?;
    ort::init_from(runtime_dll_path())
        .map_err(|error| format!("加载 ONNX Runtime {} 失败: {}", ORT_VERSION, error))?
        .commit();
    let model = UserDefinedEmbeddingModel::new(
        read_file(&directory.join("onnx/model.onnx"))?,
        TokenizerFiles {
            tokenizer_file: read_file(&directory.join("tokenizer.json"))?,
            config_file: read_file(&directory.join("config.json"))?,
            special_tokens_map_file: read_file(&directory.join("special_tokens_map.json"))?,
            tokenizer_config_file: read_file(&directory.join("tokenizer_config.json"))?,
        },
    )
    .with_pooling(Pooling::Cls);
    let mut engine = TextEmbedding::try_new_from_user_defined(
        model,
        InitOptionsUserDefined::new().with_max_length(512),
    )
    .map_err(|error| format!("创建 BGE ONNX 会话失败: {}", error))?;

    if let Ok(embeddings) = read_embedding_cache(&cache_path, &corpus_hash) {
        return Ok((engine, embeddings));
    }

    let documents = structured_search::semantic_documents();
    let mut embeddings = Vec::with_capacity(documents.len());
    for (batch_index, batch) in documents.chunks(32).enumerate() {
        let texts = batch
            .iter()
            .map(|document| document.text.as_str())
            .collect::<Vec<_>>();
        let mut batch_embeddings = engine
            .embed(texts, Some(32))
            .map_err(|error| format!("生成 UIUX 文档向量失败: {}", error))?;
        if batch_embeddings
            .iter()
            .any(|embedding| embedding.len() != MODEL_DIMENSION)
        {
            return Err("BGE 返回的文档向量维度不是 512".to_string());
        }
        embeddings.append(&mut batch_embeddings);

        let mut status = status_for(directory, "indexing", "正在建立 UIUX 语义索引");
        mark_download_complete(&mut status);
        status.indexed_documents = embeddings.len();
        status.index_progress_percent = ((batch_index + 1) * 32).min(documents.len()) as f64
            / documents.len().max(1) as f64
            * 100.0;
        let _ = write_status(&status);
    }
    write_embedding_cache(&cache_path, &corpus_hash, &embeddings)?;
    Ok((engine, embeddings))
}

fn try_acquire_lease(path: &Path, busy_message: &str) -> Result<File, String> {
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|error| format!("打开 UIUX 文件锁失败 {}: {}", path.display(), error))?;
    match lease.try_lock() {
        Ok(()) => Ok(lease),
        Err(TryLockError::WouldBlock) => Err(busy_message.to_string()),
        Err(error) => Err(format!(
            "获取 UIUX 文件锁失败 {}: {}",
            path.display(),
            error
        )),
    }
}

fn wait_for_index_lease(
    lock_path: &Path,
    cache_path: &Path,
    corpus_hash: &[u8; 32],
) -> Result<File, String> {
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    loop {
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(lock_path)
            .map_err(|error| format!("打开 UIUX 索引 lease 失败: {}", error))?;
        match lease.try_lock() {
            Ok(()) => return Ok(lease),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                if read_embedding_cache(cache_path, corpus_hash).is_ok() {
                    // 缓存已由另一个进程写完，继续等待 lease 释放后统一读取。
                    std::thread::sleep(Duration::from_millis(50));
                } else {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
            Err(TryLockError::WouldBlock) => {
                return Err("等待另一个进程建立 UIUX 语义索引超时".to_string());
            }
            Err(error) => return Err(format!("获取 UIUX 索引 lease 失败: {}", error)),
        }
    }
}

async fn ensure_ort_runtime(
    status_directory: &Path,
    proxy_config: &ProxyConfig,
) -> Result<String, String> {
    let runtime_dir = effective_runtime_dir();
    fs::create_dir_all(&runtime_dir).map_err(|error| {
        format!(
            "创建 ONNX Runtime 目录失败 {}: {}",
            runtime_dir.display(),
            error
        )
    })?;
    let lock_path = runtime_dir.join(ORT_LOCK_FILE_NAME);
    let deadline = Instant::now() + Duration::from_secs(DOWNLOAD_TIMEOUT_SECS);
    let runtime_lock = loop {
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .map_err(|error| format!("打开 ONNX Runtime 下载锁失败: {}", error))?;
        match lease.try_lock() {
            Ok(()) => break lease,
            Err(TryLockError::WouldBlock) if verify_runtime_assets().is_ok() => {
                return Ok("onnxruntime-cache".to_string())
            }
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                if CANCEL_DOWNLOAD.load(Ordering::SeqCst) {
                    return Err("用户已取消模型下载".to_string());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(TryLockError::WouldBlock) => {
                return Err("等待另一个进程下载 ONNX Runtime 超时".to_string())
            }
            Err(error) => return Err(format!("获取 ONNX Runtime 下载锁失败: {}", error)),
        }
    };

    if verify_runtime_assets().is_ok() {
        return Ok("onnxruntime-cache".to_string());
    }

    let archive_path = runtime_dir.join(ORT_ARCHIVE_FILE_NAME);
    let route_label =
        if verify_sized_sha(&archive_path, ORT_ARCHIVE_BYTES, ORT_ARCHIVE_SHA256).is_ok() {
            "onnxruntime-archive-cache".to_string()
        } else {
            if archive_path.exists() {
                fs::remove_file(&archive_path)
                    .map_err(|error| format!("移除损坏的 ONNX Runtime 归档失败: {}", error))?;
            }
            let status_directory = status_directory.to_path_buf();
            let route = download_verified_with_strategy_with_progress_and_cancel(
                ORT_ARCHIVE_URL,
                &archive_path,
                proxy_config,
                Some(ORT_ARCHIVE_SHA256),
                move |progress| {
                    let mut status = status_for(
                        &status_directory,
                        "downloading",
                        "正在下载 Microsoft ONNX Runtime",
                    );
                    status.runtime_downloaded_bytes = progress.downloaded.min(ORT_ARCHIVE_BYTES);
                    status.downloaded_bytes =
                        status.model_downloaded_bytes + status.runtime_downloaded_bytes;
                    status.progress_percent =
                        status.downloaded_bytes as f64 / DOWNLOAD_TOTAL_BYTES as f64 * 100.0;
                    let _ = write_status(&status);
                },
                || CANCEL_DOWNLOAD.load(Ordering::SeqCst),
            )
            .await?;
            route.label
        };

    verify_sized_sha(&archive_path, ORT_ARCHIVE_BYTES, ORT_ARCHIVE_SHA256)?;
    extract_runtime_archive(&archive_path, &runtime_dir)?;
    verify_runtime_assets()?;
    fs::remove_file(&archive_path)
        .map_err(|error| format!("清理 ONNX Runtime 下载归档失败: {}", error))?;
    drop(runtime_lock);
    Ok(route_label)
}

fn extract_runtime_archive(archive_path: &Path, runtime_dir: &Path) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("打开 ONNX Runtime ZIP 失败: {}", error))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("读取 ONNX Runtime ZIP 失败: {}", error))?;
    extract_runtime_entry(
        &mut archive,
        ORT_DLL_ARCHIVE_PATH,
        &runtime_dir.join(ORT_DLL_FILE_NAME),
        ORT_DLL_BYTES,
        ORT_DLL_SHA256,
    )?;
    extract_runtime_entry(
        &mut archive,
        ORT_LICENSE_ARCHIVE_PATH,
        &runtime_dir.join(ORT_LICENSE_FILE_NAME),
        ORT_LICENSE_BYTES,
        ORT_LICENSE_SHA256,
    )
}

fn extract_runtime_entry(
    archive: &mut zip::ZipArchive<File>,
    entry_name: &str,
    target: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut source = archive
        .by_name(entry_name)
        .map_err(|error| format!("ONNX Runtime ZIP 缺少 {}: {}", entry_name, error))?;
    let part = part_path_for(target);
    let mut output = File::create(&part)
        .map_err(|error| format!("创建 ONNX Runtime 临时文件失败: {}", error))?;
    std::io::copy(&mut source, &mut output)
        .map_err(|error| format!("解压 ONNX Runtime 文件失败: {}", error))?;
    output
        .flush()
        .map_err(|error| format!("刷新 ONNX Runtime 临时文件失败: {}", error))?;
    drop(output);
    verify_sized_sha(&part, expected_size, expected_sha256)?;
    if target.exists() {
        fs::remove_file(target)
            .map_err(|error| format!("替换 ONNX Runtime 文件失败: {}", error))?;
    }
    fs::rename(&part, target).map_err(|error| format!("原子替换 ONNX Runtime 文件失败: {}", error))
}

async fn download_model(directory: &Path, proxy_config: &ProxyConfig) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("创建模型目录失败 {}: {}", directory.display(), error))?;
    let lock_path = directory.join(LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|error| format!("打开模型下载锁失败: {}", error))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err("另一个 Sanshu 进程正在下载 UIUX 模型".to_string())
        }
        Err(error) => return Err(format!("获取模型下载锁失败: {}", error)),
    }

    let runtime_route = ensure_ort_runtime(directory, proxy_config).await?;
    let mut runtime_status = status_for(directory, "downloading", "ONNX Runtime 已就绪");
    runtime_status.route = Some(runtime_route);
    write_status(&runtime_status)?;

    let proxy = resolve_proxy(proxy_config).await;
    let mut completed_bytes = 0u64;
    let mut completed_files = 0usize;
    for spec in MODEL_FILES {
        if CANCEL_DOWNLOAD.load(Ordering::SeqCst) {
            return Err("用户已取消模型下载".to_string());
        }
        let target = directory.join(spec.relative_path);
        if verify_file(&target, spec).is_ok() {
            completed_bytes += spec.size;
            completed_files += 1;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建模型文件目录失败: {}", error))?;
        }
        let route = download_model_file(
            spec,
            &target,
            proxy.as_ref(),
            completed_bytes,
            completed_files,
            directory,
        )
        .await?;
        completed_bytes += spec.size;
        completed_files += 1;
        let mut status = status_for(directory, "downloading", "模型文件下载中");
        status.model_downloaded_bytes = completed_bytes;
        status.downloaded_bytes = ORT_ARCHIVE_BYTES + completed_bytes;
        status.completed_files = 1 + completed_files;
        status.progress_percent =
            status.downloaded_bytes as f64 / DOWNLOAD_TOTAL_BYTES as f64 * 100.0;
        status.route = Some(route);
        write_status(&status)?;
    }

    let mut status = status_for(directory, "verifying", "正在校验模型文件完整性");
    mark_download_complete(&mut status);
    write_status(&status)?;
    verify_model_files(directory)?;
    drop(lock);
    Ok(())
}

async fn download_model_file(
    spec: &ModelFileSpec,
    target: &Path,
    proxy: Option<&ProxyInfo>,
    completed_bytes: u64,
    completed_files: usize,
    directory: &Path,
) -> Result<String, String> {
    let official = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        MODEL_NAME, MODEL_REVISION, spec.relative_path
    );
    let mirror = format!(
        "https://hf-mirror.com/{}/resolve/{}/{}",
        MODEL_NAME, MODEL_REVISION, spec.relative_path
    );
    let mut candidates = vec![("huggingface-direct".to_string(), official.clone(), None)];
    if let Some(proxy) = proxy {
        candidates.push((
            format!("huggingface-local-proxy:{}", proxy.to_url()),
            official,
            Some(proxy.clone()),
        ));
    }
    candidates.push(("hf-mirror-direct".to_string(), mirror.clone(), None));
    if let Some(proxy) = proxy {
        candidates.push((
            format!("hf-mirror-local-proxy:{}", proxy.to_url()),
            mirror,
            Some(proxy.clone()),
        ));
    }

    let part_path = part_path_for(target);
    if verify_file(&part_path, spec).is_ok() {
        replace_file(&part_path, target, "恢复已完成的模型分片")?;
        return Ok("model-part-cache".to_string());
    }
    if fs::metadata(&part_path).is_ok_and(|metadata| metadata.len() >= spec.size) {
        fs::remove_file(&part_path)
            .map_err(|error| format!("移除损坏的模型分片失败: {}", error))?;
    }
    let mut errors = Vec::new();
    for (label, url, candidate_proxy) in candidates {
        if CANCEL_DOWNLOAD.load(Ordering::SeqCst) {
            return Err("用户已取消模型下载".to_string());
        }
        match stream_model_candidate(
            &url,
            candidate_proxy.as_ref(),
            &part_path,
            spec,
            completed_bytes,
            completed_files,
            directory,
            &label,
        )
        .await
        {
            Ok(()) => {
                if let Err(error) = verify_file(&part_path, spec) {
                    let _ = fs::remove_file(&part_path);
                    errors.push(format!("{} 校验失败: {}", label, error));
                    continue;
                }
                replace_file(&part_path, target, "替换模型文件")?;
                return Ok(label);
            }
            Err(error) => errors.push(format!("{}: {}", label, error)),
        }
    }
    Err(format!(
        "下载 {} 失败: {}",
        spec.relative_path,
        errors.join(" | ")
    ))
}

fn replace_file(source: &Path, target: &Path, operation: &str) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target).map_err(|error| format!("{}失败: {}", operation, error))?;
    }
    fs::rename(source, target).map_err(|error| format!("{}失败: {}", operation, error))
}

#[allow(clippy::too_many_arguments)]
async fn stream_model_candidate(
    url: &str,
    proxy: Option<&ProxyInfo>,
    part_path: &Path,
    spec: &ModelFileSpec,
    completed_bytes: u64,
    completed_files: usize,
    directory: &Path,
    route: &str,
) -> Result<(), String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(10));
    if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy.to_url())
                .map_err(|error| format!("创建本地代理失败: {}", error))?,
        );
    }
    let client = builder
        .build()
        .map_err(|error| format!("创建模型下载客户端失败: {}", error))?;

    let existing = fs::metadata(part_path)
        .map(|metadata| metadata.len().min(spec.size))
        .unwrap_or_default();
    let mut request = client
        .get(url)
        .header("User-Agent", concat!("sanshu/", env!("CARGO_PKG_VERSION")));
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={}-", existing));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("请求失败: {}", error))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let append = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let offset = if append { existing } else { 0 };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part_path)
        .map_err(|error| format!("打开临时模型文件失败: {}", error))?;
    let mut downloaded = offset;
    let mut stream = response.bytes_stream();
    let mut last_status = Instant::now() - Duration::from_secs(1);
    while let Some(chunk) = stream.next().await {
        if CANCEL_DOWNLOAD.load(Ordering::SeqCst) {
            return Err("用户已取消模型下载".to_string());
        }
        let chunk = chunk.map_err(|error| format!("读取下载数据失败: {}", error))?;
        file.write_all(&chunk)
            .map_err(|error| format!("写入临时模型文件失败: {}", error))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > spec.size {
            return Err(format!("下载文件超出预期大小 {}", spec.size));
        }
        if last_status.elapsed() >= Duration::from_millis(250) {
            let mut status = status_for(directory, "downloading", "模型文件下载中");
            status.model_downloaded_bytes = completed_bytes + downloaded;
            status.downloaded_bytes = ORT_ARCHIVE_BYTES + status.model_downloaded_bytes;
            status.completed_files = 1 + completed_files;
            status.progress_percent =
                status.downloaded_bytes as f64 / DOWNLOAD_TOTAL_BYTES as f64 * 100.0;
            status.route = Some(route.to_string());
            let _ = write_status(&status);
            last_status = Instant::now();
        }
    }
    file.flush()
        .map_err(|error| format!("刷新临时模型文件失败: {}", error))?;
    Ok(())
}

async fn resolve_proxy(proxy_config: &ProxyConfig) -> Option<ProxyInfo> {
    if proxy_config.enabled && !proxy_config.auto_detect {
        let proxy_type = if proxy_config.proxy_type.eq_ignore_ascii_case("socks5") {
            ProxyType::Socks5
        } else {
            ProxyType::Http
        };
        return Some(ProxyInfo::new(
            proxy_type,
            proxy_config.host.clone(),
            proxy_config.port,
        ));
    }
    if proxy_config.auto_detect || proxy_config.enabled {
        return ProxyDetector::detect_available_proxy().await;
    }
    None
}

fn status_for(directory: &Path, phase: &str, message: &str) -> UiuxModelStatus {
    let model_downloaded_bytes = quick_model_downloaded_bytes(directory);
    let runtime_downloaded_bytes = quick_runtime_downloaded_bytes();
    let downloaded_bytes = model_downloaded_bytes + runtime_downloaded_bytes;
    UiuxModelStatus {
        phase: phase.to_string(),
        model_name: MODEL_NAME.to_string(),
        revision: MODEL_REVISION.to_string(),
        model_dir: directory.to_string_lossy().to_string(),
        downloaded_bytes,
        total_bytes: DOWNLOAD_TOTAL_BYTES,
        model_downloaded_bytes,
        model_total_bytes: MODEL_TOTAL_BYTES,
        completed_files: quick_completed_files(directory),
        total_files: MODEL_FILES.len() + 1,
        runtime_version: ORT_VERSION.to_string(),
        runtime_ready: runtime_assets_have_expected_sizes(),
        runtime_dir: effective_runtime_dir().to_string_lossy().to_string(),
        runtime_downloaded_bytes,
        runtime_total_bytes: ORT_ARCHIVE_BYTES,
        indexed_documents: 0,
        total_documents: structured_search::semantic_documents().len(),
        progress_percent: downloaded_bytes as f64 / DOWNLOAD_TOTAL_BYTES as f64 * 100.0,
        index_progress_percent: 0.0,
        route: None,
        message: message.to_string(),
        error: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn current_status(directory: &Path) -> UiuxModelStatus {
    if let Ok(runtime) = RUNTIME.lock() {
        if runtime.directory.as_deref() == Some(directory) {
            match runtime.phase {
                RuntimePhase::Ready => {
                    let mut status = status_for(directory, "ready", "BGE 模型与语义索引已就绪");
                    status.indexed_documents = runtime.embeddings.len();
                    status.progress_percent = 100.0;
                    status.index_progress_percent = 100.0;
                    return status;
                }
                RuntimePhase::Loading => {
                    if let Some(status) = read_status(directory) {
                        return status;
                    }
                }
                RuntimePhase::Error => {
                    let mut status = status_for(directory, "error", "BGE 运行时初始化失败");
                    status.error = runtime.error.clone();
                    return status;
                }
                RuntimePhase::Empty => {}
            }
        }
    }
    if DOWNLOAD_RUNNING.load(Ordering::SeqCst) {
        if let Some(status) = read_status(directory) {
            return status;
        }
    }
    let mut status = status_for(directory, "missing", "模型未完整下载，auto 当前使用 BM25");
    if let Some(saved) = read_status(directory) {
        if saved.phase == "error" {
            status.error = saved.error;
        }
    }
    status
}

fn runtime_snapshot(directory: &Path) -> (RuntimePhase, Option<String>) {
    match RUNTIME.lock() {
        Ok(runtime) if runtime.directory.as_deref() == Some(directory) => {
            (runtime.phase, runtime.error.clone())
        }
        Ok(_) => (RuntimePhase::Empty, None),
        Err(error) => (RuntimePhase::Error, Some(error.to_string())),
    }
}

fn reset_runtime_if_directory_changed(directory: &Path) {
    if let Ok(runtime) = RUNTIME.lock() {
        if runtime.directory.as_deref() == Some(directory) {
            return;
        }
    }
    reset_runtime();
}

fn reset_runtime() {
    if let Ok(mut runtime) = RUNTIME.lock() {
        *runtime = RuntimeSlot::default();
    }
}

fn mark_download_complete(status: &mut UiuxModelStatus) {
    status.downloaded_bytes = DOWNLOAD_TOTAL_BYTES;
    status.model_downloaded_bytes = MODEL_TOTAL_BYTES;
    status.runtime_downloaded_bytes = ORT_ARCHIVE_BYTES;
    status.completed_files = MODEL_FILES.len() + 1;
    status.runtime_ready = true;
    status.progress_percent = 100.0;
}

fn verify_model_files(directory: &Path) -> Result<(), String> {
    for spec in MODEL_FILES {
        verify_file(&directory.join(spec.relative_path), spec)?;
    }
    Ok(())
}

fn verify_file(path: &Path, spec: &ModelFileSpec) -> Result<(), String> {
    verify_sized_sha(path, spec.size, spec.sha256)
}

fn verify_sized_sha(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("读取 {} 失败: {}", path.display(), error))?;
    if metadata.len() != expected_size {
        return Err(format!(
            "{} 大小不匹配: expected={}, actual={}",
            path.display(),
            expected_size,
            metadata.len()
        ));
    }
    let actual = sha256_file(path)?;
    if actual != expected_sha256 {
        return Err(format!("{} SHA256 不匹配", path.display()));
    }
    Ok(())
}

fn runtime_dll_path() -> PathBuf {
    effective_runtime_dir().join(ORT_DLL_FILE_NAME)
}

fn verify_runtime_assets() -> Result<(), String> {
    let runtime_dir = effective_runtime_dir();
    verify_sized_sha(
        &runtime_dir.join(ORT_DLL_FILE_NAME),
        ORT_DLL_BYTES,
        ORT_DLL_SHA256,
    )?;
    verify_sized_sha(
        &runtime_dir.join(ORT_LICENSE_FILE_NAME),
        ORT_LICENSE_BYTES,
        ORT_LICENSE_SHA256,
    )
}

fn runtime_assets_have_expected_sizes() -> bool {
    let runtime_dir = effective_runtime_dir();
    [
        (ORT_DLL_FILE_NAME, ORT_DLL_BYTES),
        (ORT_LICENSE_FILE_NAME, ORT_LICENSE_BYTES),
    ]
    .into_iter()
    .all(|(file_name, expected_size)| {
        fs::metadata(runtime_dir.join(file_name))
            .map(|metadata| metadata.len() == expected_size)
            .unwrap_or(false)
    })
}

fn assets_have_expected_sizes(directory: &Path) -> bool {
    model_files_have_expected_sizes(directory) && runtime_assets_have_expected_sizes()
}

fn model_files_have_expected_sizes(directory: &Path) -> bool {
    MODEL_FILES.iter().all(|spec| {
        fs::metadata(directory.join(spec.relative_path))
            .map(|metadata| metadata.len() == spec.size)
            .unwrap_or(false)
    })
}

fn quick_model_downloaded_bytes(directory: &Path) -> u64 {
    MODEL_FILES
        .iter()
        .map(|spec| {
            let target = directory.join(spec.relative_path);
            let final_size = fs::metadata(&target)
                .map(|value| value.len())
                .unwrap_or_default();
            if final_size == spec.size {
                return spec.size;
            }
            let part = part_path_for(&target);
            fs::metadata(part)
                .map(|value| value.len().min(spec.size))
                .unwrap_or_default()
        })
        .sum()
}

fn quick_runtime_downloaded_bytes() -> u64 {
    if runtime_assets_have_expected_sizes() {
        return ORT_ARCHIVE_BYTES;
    }
    let archive = effective_runtime_dir().join(ORT_ARCHIVE_FILE_NAME);
    let archive_size = fs::metadata(&archive)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    if archive_size == ORT_ARCHIVE_BYTES {
        return ORT_ARCHIVE_BYTES;
    }
    fs::metadata(part_path_for(&archive))
        .map(|metadata| metadata.len().min(ORT_ARCHIVE_BYTES))
        .unwrap_or_default()
}

fn quick_completed_files(directory: &Path) -> usize {
    let model_files = MODEL_FILES
        .iter()
        .filter(|spec| {
            fs::metadata(directory.join(spec.relative_path))
                .map(|metadata| metadata.len() == spec.size)
                .unwrap_or(false)
        })
        .count();
    model_files + usize::from(runtime_assets_have_expected_sizes())
}

fn part_path_for(target: &Path) -> PathBuf {
    target.with_extension(format!(
        "{}part",
        target
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{}.", value))
            .unwrap_or_default()
    ))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let file =
        File::open(path).map_err(|error| format!("打开 {} 失败: {}", path.display(), error))?;
    let mut reader = BufReader::new(file);
    let mut context = ShaContext::new(&SHA256);
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("读取 {} 失败: {}", path.display(), error))?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }
    Ok(hex::encode(context.finish().as_ref()))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("读取模型文件 {} 失败: {}", path.display(), error))
}

fn corpus_hash() -> [u8; 32] {
    let mut context = ShaContext::new(&SHA256);
    context.update(structured_search::KNOWLEDGE_VERSION.as_bytes());
    for document in structured_search::semantic_documents() {
        context.update(document.location.as_bytes());
        context.update(&[0]);
        context.update(document.text.as_bytes());
        context.update(&[0xff]);
    }
    let digest = context.finish();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(digest.as_ref());
    hash
}

fn read_embedding_cache(path: &Path, expected_hash: &[u8; 32]) -> Result<Vec<Vec<f32>>, String> {
    let mut reader = BufReader::new(
        File::open(path).map_err(|error| format!("打开语义索引缓存失败: {}", error))?,
    );
    let mut magic = [0u8; 8];
    reader
        .read_exact(&mut magic)
        .map_err(|error| error.to_string())?;
    if &magic != CACHE_MAGIC {
        return Err("语义索引缓存版本不匹配".to_string());
    }
    let mut hash = [0u8; 32];
    reader
        .read_exact(&mut hash)
        .map_err(|error| error.to_string())?;
    if &hash != expected_hash {
        return Err("UIUX 语料已变化，需要重建语义索引".to_string());
    }
    let count = read_u32(&mut reader)? as usize;
    let dimension = read_u32(&mut reader)? as usize;
    if count != structured_search::semantic_documents().len() || dimension != MODEL_DIMENSION {
        return Err("语义索引缓存形状不匹配".to_string());
    }
    let mut embeddings = Vec::with_capacity(count);
    let mut bytes = [0u8; 4];
    for _ in 0..count {
        let mut embedding = Vec::with_capacity(dimension);
        for _ in 0..dimension {
            reader
                .read_exact(&mut bytes)
                .map_err(|error| error.to_string())?;
            embedding.push(f32::from_le_bytes(bytes));
        }
        embeddings.push(embedding);
    }
    Ok(embeddings)
}

fn write_embedding_cache(
    path: &Path,
    corpus_hash: &[u8; 32],
    embeddings: &[Vec<f32>],
) -> Result<(), String> {
    let part = path.with_extension("f32.part");
    let file = File::create(&part).map_err(|error| format!("创建语义索引缓存失败: {}", error))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(CACHE_MAGIC)
        .map_err(|error| error.to_string())?;
    writer
        .write_all(corpus_hash)
        .map_err(|error| error.to_string())?;
    writer
        .write_all(&(embeddings.len() as u32).to_le_bytes())
        .map_err(|error| error.to_string())?;
    writer
        .write_all(&(MODEL_DIMENSION as u32).to_le_bytes())
        .map_err(|error| error.to_string())?;
    for embedding in embeddings {
        for value in embedding {
            writer
                .write_all(&value.to_le_bytes())
                .map_err(|error| error.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(&part, path).map_err(|error| format!("原子替换语义索引缓存失败: {}", error))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(u32::from_le_bytes(bytes))
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return -1.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    let denominator = left_norm.sqrt() * right_norm.sqrt();
    if denominator > f32::EPSILON {
        dot / denominator
    } else {
        -1.0
    }
}

fn status_path() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("sanshu").join(STATUS_FILE_NAME))
}

fn read_status(directory: &Path) -> Option<UiuxModelStatus> {
    let raw = fs::read_to_string(status_path()?).ok()?;
    let status = serde_json::from_str::<UiuxModelStatus>(&raw).ok()?;
    (Path::new(&status.model_dir) == directory).then_some(status)
}

fn write_status(status: &UiuxModelStatus) -> Result<(), String> {
    let path = status_path().ok_or_else(|| "无法确定 UIUX 模型状态目录".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 UIUX 模型状态目录失败: {}", error))?;
    }
    let part = path.with_extension(format!("json.{}.part", std::process::id()));
    let raw = serde_json::to_vec_pretty(status)
        .map_err(|error| format!("序列化 UIUX 模型状态失败: {}", error))?;
    fs::write(&part, raw).map_err(|error| format!("写入 UIUX 模型状态失败: {}", error))?;
    if path.exists() {
        fs::remove_file(&path).map_err(|error| format!("替换 UIUX 模型状态失败: {}", error))?;
    }
    fs::rename(&part, &path).map_err(|error| format!("原子写入 UIUX 模型状态失败: {}", error))
}

fn remove_known_model_files(directory: &Path) -> Result<(), String> {
    for spec in MODEL_FILES {
        let target = directory.join(spec.relative_path);
        for path in [target.clone(), part_path_for(&target)] {
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|error| format!("移除 {} 失败: {}", path.display(), error))?;
            }
        }
    }
    let cache_path = directory.join(EMBEDDING_CACHE_FILE);
    if cache_path.exists() {
        fs::remove_file(&cache_path)
            .map_err(|error| format!("移除 {} 失败: {}", cache_path.display(), error))?;
    }
    let onnx_dir = directory.join("onnx");
    if onnx_dir.is_dir() {
        let _ = fs::remove_dir(&onnx_dir);
    }
    if directory.is_dir() {
        let _ = fs::remove_dir(directory);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_manifest_has_expected_total_and_unique_paths() {
        let total = MODEL_FILES.iter().map(|spec| spec.size).sum::<u64>();
        let paths = MODEL_FILES
            .iter()
            .map(|spec| spec.relative_path)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(total, MODEL_TOTAL_BYTES);
        assert_eq!(paths.len(), MODEL_FILES.len());
        assert_eq!(DOWNLOAD_TOTAL_BYTES, 174_089_011);
        assert_eq!(ORT_ARCHIVE_SHA256.len(), 64);
    }

    #[test]
    fn cosine_similarity_handles_normalized_and_empty_vectors() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.0001);
        assert_eq!(cosine_similarity(&[], &[]), -1.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 0.0]), -1.0);
    }

    #[tokio::test]
    async fn completed_model_part_is_promoted_without_network() {
        let directory = tempfile::tempdir().expect("应创建模型分片测试目录");
        let target = directory.path().join("model.bin");
        let part = part_path_for(&target);
        fs::write(&part, b"test").expect("应写入完整测试分片");
        let spec = ModelFileSpec {
            relative_path: "model.bin",
            size: 4,
            sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        };

        let route = download_model_file(&spec, &target, None, 0, 0, directory.path())
            .await
            .expect("完整分片应直接转为正式文件");

        assert_eq!(route, "model-part-cache");
        assert_eq!(fs::read(&target).expect("应读取正式文件"), b"test");
        assert!(!part.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "下载约 174 MB 固定资产并建立完整 UIUX 向量索引"]
    async fn model_download_index_and_query_e2e() {
        let directory = std::env::var_os("SANSHU_UIUX_MODEL_E2E_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| effective_model_dir(None));
        download_model(&directory, &crate::config::default_proxy_config())
            .await
            .expect("固定模型与运行时应完成下载和校验");

        let (mut model, embeddings) =
            load_runtime(&directory).expect("BGE 运行时应完成加载和建索引");
        assert_eq!(
            embeddings.len(),
            structured_search::semantic_documents().len()
        );
        assert!(embeddings
            .iter()
            .all(|embedding| embedding.len() == MODEL_DIMENSION));

        let query = format!(
            "{}{}",
            QUERY_PREFIX, "黑洞吞噬星球、地月卫星公转、深邃多层星空与暗手抓握地球的鼠标动效"
        );
        let query_embedding = model
            .embed(vec![query], Some(1))
            .expect("BGE 应生成查询向量")
            .pop()
            .expect("BGE 应返回一条查询向量");
        let top_score = embeddings
            .iter()
            .map(|embedding| cosine_similarity(&query_embedding, embedding))
            .max_by(f32::total_cmp)
            .expect("语义索引应包含文档");
        assert!(top_score.is_finite() && top_score > 0.0);

        let semantic_only_query =
            format!("{}{}", QUERY_PREFIX, "用醒目卡片展示健身训练进度和健康指标");
        let unrelated_query = format!("{}{}", QUERY_PREFIX, "如何烹饪红烧肉并计算卡路里");
        let scores = model
            .embed(vec![semantic_only_query, unrelated_query], Some(2))
            .expect("BGE 应生成质量门槛查询向量")
            .into_iter()
            .map(|query| {
                embeddings
                    .iter()
                    .map(|embedding| cosine_similarity(&query, embedding))
                    .max_by(f32::total_cmp)
                    .expect("语义索引应包含文档")
            })
            .collect::<Vec<_>>();
        assert!(scores[0] >= super::super::semantic_search::SEMANTIC_ONLY_MIN_SCORE);
        assert!(scores[1] < super::super::semantic_search::SEMANTIC_ONLY_MIN_SCORE);
        println!(
            "UIUX BGE E2E: directory={}, documents={}, top_score={:.4}, semantic_only={:.4}, unrelated={:.4}",
            directory.display(),
            embeddings.len(),
            top_score,
            scores[0],
            scores[1]
        );
    }
}
