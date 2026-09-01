//! Accurate 模式使用的固定 BGE cross-encoder 资产、下载状态与进程内运行时。

use fastembed::{
    RerankInitOptionsUserDefined, TextRerank, TokenizerFiles, UserDefinedRerankingModel,
};
use once_cell::sync::Lazy;
use ring::digest::{Context as ShaContext, SHA256};
use serde::Serialize;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::ProxyConfig;
use crate::network::download_verified_with_direct_or_local_proxy_with_progress_and_cancel;

pub const MODEL_NAME: &str = "BAAI/bge-reranker-base";
pub const MODEL_REVISION: &str = "580465186bcc87f862a9b2f9003d720af2377980";
pub const INTRA_THREADS: usize = 8;
pub const BATCH_SIZE: usize = 8;
pub const MAX_LENGTH: usize = 512;

const MODEL_TOTAL_BYTES: u64 = 1_134_628_267;
const LOCK_FILE_NAME: &str = ".sou-reranker.lock";
const ORT_DLL_FILE_NAME: &str = "onnxruntime.dll";
const ORT_DLL_BYTES: u64 = 15_809_848;

#[derive(Clone, Copy)]
struct ModelFileSpec {
    relative_path: &'static str,
    size: u64,
    sha256: &'static str,
}

const MODEL_FILES: &[ModelFileSpec] = &[
    ModelFileSpec {
        relative_path: "onnx/model.onnx",
        size: 1_112_459_588,
        sha256: "15b9a8c3da82eddf263df571281166e00e9308fe19d077084b642ebfcaf06d2b",
    },
    ModelFileSpec {
        relative_path: "sentencepiece.bpe.model",
        size: 5_069_051,
        sha256: "cfc8146abe2a0488e9e2a0c56de7952f7c11ab059eca145a0a727afce0db2865",
    },
    ModelFileSpec {
        relative_path: "special_tokens_map.json",
        size: 279,
        sha256: "d5469a60db23249c7f8945013d78df30b44b6bf686c6bb4740f4223f77b1b535",
    },
    ModelFileSpec {
        relative_path: "tokenizer.json",
        size: 17_098_107,
        sha256: "9eb652ac4e40cc093272bbbe0f55d521cf67570060227109b5cdc20945a4489e",
    },
    ModelFileSpec {
        relative_path: "tokenizer_config.json",
        size: 443,
        sha256: "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    },
    ModelFileSpec {
        relative_path: "config.json",
        size: 799,
        sha256: "289adf7ada1eb6b4afa7589a48a032d45a076cf2e46dcdb3b4cabc33be14f708",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePhase {
    Missing,
    Loading,
    Ready,
    Error,
}

impl RuntimePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub phase: RuntimePhase,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RerankerModelStatus {
    pub phase: String,
    pub model_name: String,
    pub revision: String,
    pub model_dir: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub completed_files: usize,
    pub total_files: usize,
    pub progress_percent: f64,
    pub runtime_ready: bool,
    pub runtime_threads: usize,
    pub batch_size: usize,
    pub route: Option<String>,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RerankMatch {
    pub index: usize,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct RerankerUnavailable {
    pub state: String,
    pub message: String,
}

struct RuntimeSlot {
    directory: Option<PathBuf>,
    phase: RuntimePhase,
    model: Option<TextRerank>,
    error: Option<String>,
}

impl Default for RuntimeSlot {
    fn default() -> Self {
        Self {
            directory: None,
            phase: RuntimePhase::Missing,
            model: None,
            error: None,
        }
    }
}

static RUNTIME: Lazy<Mutex<RuntimeSlot>> = Lazy::new(|| Mutex::new(RuntimeSlot::default()));
static DOWNLOAD_RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL_DOWNLOAD: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_STATUS: Lazy<Mutex<Option<RerankerModelStatus>>> = Lazy::new(|| Mutex::new(None));

pub fn assets_have_expected_sizes(directory: &Path) -> bool {
    MODEL_FILES.iter().all(|spec| {
        fs::metadata(directory.join(spec.relative_path))
            .map(|metadata| metadata.is_file() && metadata.len() == spec.size)
            .unwrap_or(false)
    })
}

pub fn runtime_snapshot(directory: &Path) -> RuntimeSnapshot {
    match RUNTIME.lock() {
        Ok(runtime) if runtime.directory.as_deref() == Some(directory) => RuntimeSnapshot {
            phase: runtime.phase,
            error: runtime.error.clone(),
        },
        Ok(_) => RuntimeSnapshot {
            phase: RuntimePhase::Missing,
            error: None,
        },
        Err(error) => RuntimeSnapshot {
            phase: RuntimePhase::Error,
            error: Some(error.to_string()),
        },
    }
}

pub fn current_status(directory: &Path) -> RerankerModelStatus {
    if DOWNLOAD_RUNNING.load(Ordering::SeqCst) {
        if let Ok(status) = DOWNLOAD_STATUS.lock() {
            if let Some(status) = status.as_ref() {
                if Path::new(&status.model_dir) == directory {
                    return status.clone();
                }
            }
        }
    }

    let (downloaded_bytes, completed_files) = quick_download_progress(directory);
    let runtime_ready = ort_runtime_ready();
    if !assets_have_expected_sizes(directory) {
        return status_for(
            directory,
            "missing",
            downloaded_bytes,
            completed_files,
            runtime_ready,
            "准确模式模型资产尚未下载完整",
            None,
        );
    }

    match runtime_snapshot(directory) {
        RuntimeSnapshot {
            phase: RuntimePhase::Loading,
            ..
        } => status_for(
            directory,
            "loading",
            MODEL_TOTAL_BYTES,
            MODEL_FILES.len(),
            runtime_ready,
            "准确模式模型正在按需加载",
            None,
        ),
        RuntimeSnapshot {
            phase: RuntimePhase::Error,
            error,
        } => status_for(
            directory,
            "error",
            MODEL_TOTAL_BYTES,
            MODEL_FILES.len(),
            runtime_ready,
            "准确模式模型运行时初始化失败",
            error,
        ),
        _ => status_for(
            directory,
            "ready",
            MODEL_TOTAL_BYTES,
            MODEL_FILES.len(),
            runtime_ready,
            "准确模式模型资产已就绪，将在首次查询时加载",
            None,
        ),
    }
}

pub fn start_download(
    directory: PathBuf,
    proxy_config: ProxyConfig,
) -> Result<RerankerModelStatus, String> {
    if DOWNLOAD_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("准确模式模型下载任务已在运行".to_string());
    }
    CANCEL_DOWNLOAD.store(false, Ordering::SeqCst);
    let (downloaded_bytes, completed_files) = quick_download_progress(&directory);
    let initial = status_for(
        &directory,
        "downloading",
        downloaded_bytes,
        completed_files,
        ort_runtime_ready(),
        "准备下载准确模式模型",
        None,
    );
    write_download_status(initial.clone());

    tokio::spawn(async move {
        let result = download_assets(&directory, &proxy_config).await;
        DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(route) => {
                let mut status = status_for(
                    &directory,
                    "ready",
                    MODEL_TOTAL_BYTES,
                    MODEL_FILES.len(),
                    ort_runtime_ready(),
                    "准确模式模型下载与校验完成",
                    None,
                );
                status.route = route;
                write_download_status(status);
            }
            Err(error) => {
                let cancelled = CANCEL_DOWNLOAD.load(Ordering::SeqCst);
                let (downloaded_bytes, completed_files) = quick_download_progress(&directory);
                let status = status_for(
                    &directory,
                    if cancelled { "missing" } else { "error" },
                    downloaded_bytes,
                    completed_files,
                    ort_runtime_ready(),
                    if cancelled {
                        "准确模式模型下载已取消"
                    } else {
                        "准确模式模型下载失败"
                    },
                    (!cancelled).then_some(error.clone()),
                );
                write_download_status(status);
                log::warn!("[sou-reranker] 模型下载结束，error={}", error);
            }
        }
    });
    Ok(initial)
}

pub fn cancel_download() {
    CANCEL_DOWNLOAD.store(true, Ordering::SeqCst);
}

pub fn remove_assets(directory: &Path) -> Result<RerankerModelStatus, String> {
    if DOWNLOAD_RUNNING.load(Ordering::SeqCst) {
        return Err("请先取消正在运行的准确模式模型下载".to_string());
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("创建准确模式模型目录失败: {}", error))?;
    let lock_path = directory.join(LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|error| format!("打开准确模式模型文件锁失败: {}", error))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err("另一个 Sanshu 进程正在使用准确模式模型目录".to_string())
        }
        Err(error) => return Err(format!("获取准确模式模型文件锁失败: {}", error)),
    }

    release();
    for spec in MODEL_FILES {
        let target = directory.join(spec.relative_path);
        for path in [target.clone(), partial_path(&target)] {
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|error| format!("移除 {} 失败: {}", path.display(), error))?;
            }
        }
    }
    let status = status_for(
        directory,
        "missing",
        0,
        0,
        ort_runtime_ready(),
        "准确模式模型已移除，共享 ONNX Runtime 保留",
        None,
    );
    write_download_status(status.clone());
    Ok(status)
}

pub fn ensure_started(directory: &Path) {
    let directory = directory.to_path_buf();
    let mut runtime = match RUNTIME.lock() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::warn!("[sou-reranker] 锁定模型运行时失败: {}", error);
            return;
        }
    };
    if runtime.directory.as_deref() == Some(directory.as_path())
        && matches!(runtime.phase, RuntimePhase::Loading | RuntimePhase::Ready)
    {
        return;
    }
    if !assets_have_expected_sizes(&directory) || !ort_runtime_ready() {
        runtime.directory = Some(directory);
        runtime.phase = RuntimePhase::Missing;
        runtime.model = None;
        runtime.error = None;
        return;
    }

    runtime.directory = Some(directory.clone());
    runtime.phase = RuntimePhase::Loading;
    runtime.model = None;
    runtime.error = None;
    drop(runtime);

    std::thread::spawn(move || match load_model(&directory) {
        Ok(model) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path()) {
                    runtime.model = Some(model);
                    runtime.phase = RuntimePhase::Ready;
                    runtime.error = None;
                }
            }
            log::info!("[sou-reranker] BGE reranker 运行时已就绪");
        }
        Err(error) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path()) {
                    runtime.phase = RuntimePhase::Error;
                    runtime.error = Some(error.clone());
                }
            }
            log::warn!("[sou-reranker] BGE reranker 初始化失败: {}", error);
        }
    });
}

pub async fn rerank(
    directory: &Path,
    query: &str,
    documents: Vec<String>,
    wait_budget: Duration,
) -> Result<Vec<RerankMatch>, RerankerUnavailable> {
    let started_at = Instant::now();
    ensure_started(directory);
    wait_until_ready(directory, wait_budget).await?;
    let remaining = wait_budget.saturating_sub(started_at.elapsed());
    if remaining.is_zero() {
        return Err(RerankerUnavailable {
            state: "timeout".to_string(),
            message: "准确模式重排等待已用完查询预算".to_string(),
        });
    }
    let directory = directory.to_path_buf();
    let query = query.to_string();
    let task = tokio::task::spawn_blocking(move || rerank_ready(&directory, &query, documents));
    match tokio::time::timeout(remaining, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(RerankerUnavailable {
            state: "error".to_string(),
            message: format!("等待准确模式重排任务失败: {}", error),
        }),
        Err(_) => Err(RerankerUnavailable {
            state: "timeout".to_string(),
            message: "准确模式重排超过查询预算".to_string(),
        }),
    }
}

pub fn release() {
    if let Ok(mut runtime) = RUNTIME.lock() {
        *runtime = RuntimeSlot::default();
    }
}

async fn wait_until_ready(
    directory: &Path,
    wait_budget: Duration,
) -> Result<(), RerankerUnavailable> {
    let deadline = Instant::now() + wait_budget;
    loop {
        match runtime_snapshot(directory) {
            RuntimeSnapshot {
                phase: RuntimePhase::Ready,
                ..
            } => return Ok(()),
            RuntimeSnapshot {
                phase: RuntimePhase::Missing,
                ..
            } => {
                return Err(RerankerUnavailable {
                    state: "missing".to_string(),
                    message: "准确模式模型或共享 ONNX Runtime 尚未就绪".to_string(),
                })
            }
            RuntimeSnapshot {
                phase: RuntimePhase::Error,
                error,
            } => {
                return Err(RerankerUnavailable {
                    state: "error".to_string(),
                    message: error.unwrap_or_else(|| "准确模式模型初始化失败".to_string()),
                })
            }
            RuntimeSnapshot {
                phase: RuntimePhase::Loading,
                ..
            } if Instant::now() >= deadline => {
                return Err(RerankerUnavailable {
                    state: "loading".to_string(),
                    message: "准确模式模型仍在加载".to_string(),
                })
            }
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

fn rerank_ready(
    directory: &Path,
    query: &str,
    documents: Vec<String>,
) -> Result<Vec<RerankMatch>, RerankerUnavailable> {
    let mut runtime = RUNTIME.lock().map_err(|error| RerankerUnavailable {
        state: "error".to_string(),
        message: format!("锁定准确模式模型失败: {}", error),
    })?;
    if runtime.directory.as_deref() != Some(directory) || runtime.phase != RuntimePhase::Ready {
        return Err(RerankerUnavailable {
            state: "loading".to_string(),
            message: "准确模式模型运行时尚未就绪".to_string(),
        });
    }
    let model = runtime.model.as_mut().ok_or_else(|| RerankerUnavailable {
        state: "error".to_string(),
        message: "准确模式模型实例缺失".to_string(),
    })?;
    let document_refs = documents.iter().map(String::as_str).collect::<Vec<_>>();
    model
        .rerank(query, &document_refs, false, Some(BATCH_SIZE))
        .map(|results| {
            results
                .into_iter()
                .map(|result| RerankMatch {
                    index: result.index,
                    score: result.score,
                })
                .collect()
        })
        .map_err(|error| RerankerUnavailable {
            state: "error".to_string(),
            message: format!("准确模式模型推理失败: {}", error),
        })
}

fn load_model(directory: &Path) -> Result<TextRerank, String> {
    verify_assets(directory)?;
    ort::init_from(crate::mcp::embedding::runtime_dir().join(ORT_DLL_FILE_NAME))
        .map_err(|error| format!("加载共享 ONNX Runtime 失败: {}", error))?
        .commit();
    let model = UserDefinedRerankingModel::new(
        directory.join("onnx/model.onnx"),
        TokenizerFiles {
            tokenizer_file: read_file(&directory.join("tokenizer.json"))?,
            config_file: read_file(&directory.join("config.json"))?,
            special_tokens_map_file: read_file(&directory.join("special_tokens_map.json"))?,
            tokenizer_config_file: read_file(&directory.join("tokenizer_config.json"))?,
        },
    );
    let mut model = TextRerank::try_new_from_user_defined(
        model,
        RerankInitOptionsUserDefined::new()
            .with_max_length(MAX_LENGTH)
            .with_intra_threads(INTRA_THREADS),
    )
    .map_err(|error| format!("创建 BGE reranker ONNX 会话失败: {}", error))?;

    // 中文说明：固定 batch 预热一次 ONNX 图，避免首条真实查询承担算子初始化长尾。
    let warmup_documents = (0..BATCH_SIZE)
        .map(|index| format!("准确模式预热文档 {index}：代码上下文与业务流程"))
        .collect::<Vec<_>>();
    let warmup_refs = warmup_documents
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    model
        .rerank(
            "准确模式预热查询：定位相关代码实现",
            &warmup_refs,
            false,
            Some(BATCH_SIZE),
        )
        .map_err(|error| format!("准确模式模型预热推理失败: {}", error))?;
    Ok(model)
}

async fn download_assets(
    directory: &Path,
    proxy_config: &ProxyConfig,
) -> Result<Option<String>, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("创建准确模式模型目录失败: {}", error))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(directory.join(LOCK_FILE_NAME))
        .map_err(|error| format!("打开准确模式模型下载锁失败: {}", error))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err("另一个 Sanshu 进程正在下载准确模式模型".to_string())
        }
        Err(error) => return Err(format!("获取准确模式模型下载锁失败: {}", error)),
    }

    let mut completed_bytes = 0u64;
    let mut completed_files = 0usize;
    let mut last_route = None;
    for spec in MODEL_FILES {
        if CANCEL_DOWNLOAD.load(Ordering::SeqCst) {
            return Err("用户已取消准确模式模型下载".to_string());
        }
        let target = directory.join(spec.relative_path);
        if verify_file(&target, spec).is_ok() {
            completed_bytes += spec.size;
            completed_files += 1;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建准确模式模型文件目录失败: {}", error))?;
        }
        let part = partial_path(&target);
        if verify_file(&part, spec).is_ok() {
            replace_file(&part, &target)?;
            completed_bytes += spec.size;
            completed_files += 1;
            continue;
        }

        let official = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            MODEL_NAME, MODEL_REVISION, spec.relative_path
        );
        let mirror = format!(
            "https://hf-mirror.com/{}/resolve/{}/{}",
            MODEL_NAME, MODEL_REVISION, spec.relative_path
        );
        let mut errors = Vec::new();
        let mut downloaded = None;
        for (label, url) in [("huggingface", official), ("hf-mirror", mirror)] {
            let status_directory = directory.to_path_buf();
            let base_bytes = completed_bytes;
            let base_files = completed_files;
            match download_verified_with_direct_or_local_proxy_with_progress_and_cancel(
                &url,
                &target,
                proxy_config,
                Some(spec.sha256),
                move |progress| {
                    let current = base_bytes + progress.downloaded.min(spec.size);
                    let mut status = status_for(
                        &status_directory,
                        "downloading",
                        current,
                        base_files,
                        ort_runtime_ready(),
                        "准确模式模型文件下载中",
                        None,
                    );
                    status.route = Some(label.to_string());
                    write_download_status(status);
                },
                || CANCEL_DOWNLOAD.load(Ordering::SeqCst),
            )
            .await
            {
                Ok(route) => {
                    downloaded = Some(route.label);
                    break;
                }
                Err(error) => errors.push(format!("{}: {}", label, error)),
            }
        }
        let route = downloaded.ok_or_else(|| {
            format!(
                "下载准确模式模型文件 {} 失败: {}",
                spec.relative_path,
                errors.join(" | ")
            )
        })?;
        completed_bytes += spec.size;
        completed_files += 1;
        last_route = Some(route);
    }

    write_download_status(status_for(
        directory,
        "verifying",
        MODEL_TOTAL_BYTES,
        MODEL_FILES.len(),
        ort_runtime_ready(),
        "正在校验准确模式模型完整性",
        None,
    ));
    verify_assets(directory)?;
    Ok(last_route)
}

fn verify_assets(directory: &Path) -> Result<(), String> {
    for spec in MODEL_FILES {
        verify_file(&directory.join(spec.relative_path), spec)?;
    }
    Ok(())
}

fn verify_file(path: &Path, spec: &ModelFileSpec) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("读取模型文件 {} 失败: {}", path.display(), error))?;
    if !metadata.is_file() || metadata.len() != spec.size {
        return Err(format!(
            "模型文件 {} 大小不匹配: expected={}, actual={}",
            path.display(),
            spec.size,
            metadata.len()
        ));
    }
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(spec.sha256) {
        return Err(format!("模型文件 {} SHA256 不匹配", path.display()));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("打开模型文件 {} 失败: {}", path.display(), error))?;
    let mut context = ShaContext::new(&SHA256);
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取模型文件 {} 失败: {}", path.display(), error))?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect())
}

fn quick_download_progress(directory: &Path) -> (u64, usize) {
    let mut bytes = 0u64;
    let mut completed = 0usize;
    for spec in MODEL_FILES {
        let target = directory.join(spec.relative_path);
        if fs::metadata(&target).is_ok_and(|metadata| metadata.len() == spec.size) {
            bytes += spec.size;
            completed += 1;
            continue;
        }
        bytes += fs::metadata(partial_path(&target))
            .map(|metadata| metadata.len().min(spec.size))
            .unwrap_or_default();
    }
    (bytes, completed)
}

fn status_for(
    directory: &Path,
    phase: &str,
    downloaded_bytes: u64,
    completed_files: usize,
    runtime_ready: bool,
    message: &str,
    error: Option<String>,
) -> RerankerModelStatus {
    RerankerModelStatus {
        phase: phase.to_string(),
        model_name: MODEL_NAME.to_string(),
        revision: MODEL_REVISION.to_string(),
        model_dir: directory.to_string_lossy().to_string(),
        downloaded_bytes,
        total_bytes: MODEL_TOTAL_BYTES,
        completed_files,
        total_files: MODEL_FILES.len(),
        progress_percent: downloaded_bytes as f64 / MODEL_TOTAL_BYTES as f64 * 100.0,
        runtime_ready,
        runtime_threads: INTRA_THREADS,
        batch_size: BATCH_SIZE,
        route: None,
        message: message.to_string(),
        error,
    }
}

fn write_download_status(status: RerankerModelStatus) {
    if let Ok(mut current) = DOWNLOAD_STATUS.lock() {
        *current = Some(status);
    }
}

fn ort_runtime_ready() -> bool {
    fs::metadata(crate::mcp::embedding::runtime_dir().join(ORT_DLL_FILE_NAME))
        .map(|metadata| metadata.is_file() && metadata.len() == ORT_DLL_BYTES)
        .unwrap_or(false)
}

fn partial_path(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("model");
    target.with_file_name(format!("{}.part", file_name))
}

fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target)
            .map_err(|error| format!("移除旧模型文件 {} 失败: {}", target.display(), error))?;
    }
    fs::rename(source, target)
        .map_err(|error| format!("原子替换模型文件 {} 失败: {}", target.display(), error))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("读取模型文件 {} 失败: {}", path.display(), error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_fixed_total_and_unique_paths() {
        let total = MODEL_FILES.iter().map(|spec| spec.size).sum::<u64>();
        let paths = MODEL_FILES
            .iter()
            .map(|spec| spec.relative_path)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(total, MODEL_TOTAL_BYTES);
        assert_eq!(paths.len(), MODEL_FILES.len());
        assert!(MODEL_FILES.iter().all(|spec| spec.sha256.len() == 64));
    }

    #[test]
    fn missing_directory_reports_partial_progress_without_loading_runtime() {
        let directory = tempfile::tempdir().expect("reranker 状态测试目录应创建成功");
        let status = current_status(directory.path());
        assert_eq!(status.phase, "missing");
        assert_eq!(status.downloaded_bytes, 0);
        assert_eq!(
            runtime_snapshot(directory.path()).phase,
            RuntimePhase::Missing
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "下载并校验约 1.13 GB 固定模型资产，再执行真实 cross-encoder 推理"]
    async fn model_download_and_rerank_e2e() {
        let directory = std::env::var_os("SANSHU_SOU_RERANKER_E2E_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(crate::config::default_sou_reranker_model_dir);
        download_assets(&directory, &crate::config::default_proxy_config())
            .await
            .expect("固定 reranker 模型应完成下载与校验");
        ensure_started(&directory);
        let results = rerank(
            &directory,
            "工作流阶段切换时补全前序步骤的起止时刻",
            vec![
                "OrderProgressTimingService 更新工作流步骤开始与结束时间".to_string(),
                "应用启动时读取主题颜色配置".to_string(),
            ],
            Duration::from_secs(180),
        )
        .await
        .expect("固定 reranker 应完成真实推理");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 0);
        assert!(results.iter().all(|result| result.score.is_finite()));
        release();
    }
}
