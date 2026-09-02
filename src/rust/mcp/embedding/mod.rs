//! BGE 本地向量模型的共享进程内推理运行时。

use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use once_cell::sync::Lazy;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MODEL_NAME: &str = "Xenova/bge-small-zh-v1.5";
pub const MODEL_REVISION: &str = "75c43b069aac4d136ba6bc1122f995fedcfd2781";
pub const MODEL_DIMENSION: usize = 512;
pub const QUERY_PREFIX: &str = "为这个句子生成表示以用于检索相关文章：";
pub const ORT_VERSION: &str = "1.28.0";
// 中文说明：本机 512/1024 分块受控基准中 64 优于 32/128，固定为中间值控制 CPU 峰值与显存占用。
pub const EMBEDDING_BATCH_SIZE: usize = 64;

const MODEL_FILES: &[(&str, u64)] = &[
    ("onnx/model.onnx", 94_851_877),
    ("config.json", 716),
    ("special_tokens_map.json", 125),
    ("tokenizer.json", 439_125),
    ("tokenizer_config.json", 367),
];
const ORT_DLL_FILE_NAME: &str = "onnxruntime.dll";
const ORT_DLL_BYTES: u64 = 15_809_848;
#[cfg(feature = "cuda")]
const CUDA_PROVIDER_DLL_FILE_NAME: &str = "onnxruntime_providers_cuda.dll";
#[cfg(feature = "cuda")]
const CUDA_RUNTIME_ENV: &str = "SANSHU_ORT_CUDA_DIR";
const CUDA_INTRA_THREADS: usize = 4;
const CUDA_PREFLIGHT_CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPreference {
    Auto,
    Cuda,
    Cpu,
}

impl Default for ProviderPreference {
    fn default() -> Self {
        Self::Auto
    }
}

impl ProviderPreference {
    pub fn from_config(configured: Option<&str>) -> Self {
        match crate::config::effective_sou_embedding_provider(configured) {
            crate::config::SOU_EMBEDDING_PROVIDER_CUDA => Self::Cuda,
            crate::config::SOU_EMBEDDING_PROVIDER_CPU => Self::Cpu,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => crate::config::SOU_EMBEDDING_PROVIDER_AUTO,
            Self::Cuda => crate::config::SOU_EMBEDDING_PROVIDER_CUDA,
            Self::Cpu => crate::config::SOU_EMBEDDING_PROVIDER_CPU,
        }
    }

    fn attempts_cuda(self) -> bool {
        matches!(self, Self::Auto | Self::Cuda)
    }

    fn allows_cpu_fallback(self) -> bool {
        self == Self::Auto
    }
}

#[derive(Debug, Clone)]
pub struct OrtRuntimeInfo {
    /// 实际载入的 ORT 核心 DLL；进程内只能选择一次。
    pub runtime_path: PathBuf,
    /// 核心 DLL 是否来自带 CUDA provider 的运行时包。
    pub cuda_capable: bool,
}

#[derive(Debug, Clone)]
pub struct CudaRuntimeStatus {
    /// 找到的 CUDA 包目录；即使预检失败也保留，便于界面定位安装路径。
    pub directory: Option<PathBuf>,
    /// provider DLL 或其依赖加载失败的具体原因。
    pub error: Option<String>,
}

impl CudaRuntimeStatus {
    fn available(&self) -> bool {
        self.directory.is_some() && self.error.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionProvider {
    Cpu,
    Cuda,
}

impl ExecutionProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => crate::config::SOU_EMBEDDING_PROVIDER_CPU,
            Self::Cuda => crate::config::SOU_EMBEDDING_PROVIDER_CUDA,
        }
    }
}

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
    pub model_dir: PathBuf,
    pub error: Option<String>,
    pub requested_provider: String,
    pub execution_provider: String,
    pub provider_fallback_reason: Option<String>,
    pub cuda_runtime_available: bool,
    pub cuda_runtime_dir: Option<PathBuf>,
    pub cuda_runtime_error: Option<String>,
    pub runtime_path: Option<PathBuf>,
    pub batch_size: usize,
    pub intra_threads: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct EmbeddingUnavailable {
    pub state: String,
    pub message: String,
}

struct LoadedModel {
    model: TextEmbedding,
    execution_provider: ExecutionProvider,
    provider_fallback_reason: Option<String>,
    runtime_path: PathBuf,
    intra_threads: usize,
}

struct RuntimeSlot {
    directory: Option<PathBuf>,
    phase: RuntimePhase,
    model: Option<TextEmbedding>,
    error: Option<String>,
    requested_provider: ProviderPreference,
    execution_provider: ExecutionProvider,
    provider_fallback_reason: Option<String>,
    runtime_path: Option<PathBuf>,
    intra_threads: Option<usize>,
}

impl Default for RuntimeSlot {
    fn default() -> Self {
        Self {
            directory: None,
            phase: RuntimePhase::Missing,
            model: None,
            error: None,
            requested_provider: ProviderPreference::Auto,
            execution_provider: ExecutionProvider::Cpu,
            provider_fallback_reason: None,
            runtime_path: None,
            intra_threads: None,
        }
    }
}

static RUNTIME: Lazy<Mutex<RuntimeSlot>> = Lazy::new(|| Mutex::new(RuntimeSlot::default()));

#[derive(Default)]
struct ProviderConfigCache {
    initialized: bool,
    value: ProviderPreference,
}

struct CudaPreflightCache {
    checked_at: Instant,
    status: CudaRuntimeStatus,
}

struct OrtRuntimeState {
    info: Option<OrtRuntimeInfo>,
}

static PROVIDER_CONFIG: Lazy<Mutex<ProviderConfigCache>> =
    Lazy::new(|| Mutex::new(ProviderConfigCache::default()));
static CUDA_PREFLIGHT: Lazy<Mutex<Option<CudaPreflightCache>>> = Lazy::new(|| Mutex::new(None));
static ORT_RUNTIME: Lazy<Mutex<OrtRuntimeState>> =
    Lazy::new(|| Mutex::new(OrtRuntimeState { info: None }));

pub fn effective_model_dir(shared: Option<&str>, legacy_uiux: Option<&str>) -> PathBuf {
    shared
        .or(legacy_uiux)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_model_dir)
}

pub fn default_model_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("sanshu")
        .join("models")
        .join("bge-small-zh-v1.5")
}

pub fn runtime_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("sanshu")
        .join("runtimes")
        .join(format!("onnxruntime-{}", ORT_VERSION))
}

pub fn configured_provider() -> ProviderPreference {
    let mut cache = match PROVIDER_CONFIG.lock() {
        Ok(cache) => cache,
        Err(_) => return ProviderPreference::default(),
    };
    if !cache.initialized {
        cache.value = crate::config::load_standalone_config()
            .ok()
            .and_then(|config| config.mcp_config.sou_embedding_provider)
            .map(|value| ProviderPreference::from_config(Some(&value)))
            .unwrap_or_default();
        cache.initialized = true;
    }
    cache.value
}

/// 配置保存成功后同步内存值，避免状态轮询反复解析整份配置文件。
pub fn set_configured_provider(provider: ProviderPreference) {
    if let Ok(mut cache) = PROVIDER_CONFIG.lock() {
        cache.value = provider;
        cache.initialized = true;
    }
}

pub fn model_assets_have_expected_sizes(directory: &Path) -> bool {
    MODEL_FILES.iter().all(|(relative_path, size)| {
        fs::metadata(directory.join(relative_path))
            .map(|metadata| metadata.is_file() && metadata.len() == *size)
            .unwrap_or(false)
    })
}

pub fn cpu_runtime_assets_have_expected_sizes() -> bool {
    fs::metadata(runtime_dir().join(ORT_DLL_FILE_NAME))
        .map(|metadata| metadata.is_file() && metadata.len() == ORT_DLL_BYTES)
        .unwrap_or(false)
}

#[cfg(feature = "cuda")]
fn cuda_runtime_assets_have_expected_sizes(directory: &Path) -> bool {
    [ORT_DLL_FILE_NAME, CUDA_PROVIDER_DLL_FILE_NAME]
        .into_iter()
        .all(|file_name| {
            fs::metadata(directory.join(file_name))
                .map(|metadata| metadata.is_file() && metadata.len() > 0)
                .unwrap_or(false)
        })
}

/// 只检查候选目录中的固定文件，真正的依赖加载由 `cuda_runtime_status` 负责。
fn cuda_runtime_candidate_dir() -> Option<PathBuf> {
    #[cfg(not(feature = "cuda"))]
    {
        return None;
    }

    #[cfg(feature = "cuda")]
    {
        let root = runtime_dir();
        let mut candidates = Vec::new();
        if let Some(value) = std::env::var_os(CUDA_RUNTIME_ENV) {
            let directory = PathBuf::from(value);
            candidates.push(directory.clone());
            candidates.push(directory.join("lib"));
        }
        for directory in [
            root.join("cuda13"),
            root.join("cuda12"),
            root.join("cuda"),
            root.clone(),
        ] {
            candidates.push(directory.clone());
            candidates.push(directory.join("lib"));
        }
        candidates
            .into_iter()
            .find(|directory| cuda_runtime_assets_have_expected_sizes(directory))
    }
}

/// 查找可由 ORT 动态加载的 CUDA 包；安装目录通过环境变量或共享运行时子目录提供。
pub fn cuda_runtime_dir() -> Option<PathBuf> {
    cuda_runtime_candidate_dir()
}

/// 预加载 CUDA provider，让 Windows loader 同时校验 provider 及其依赖 DLL。
/// 结果短暂缓存，避免 UI 状态轮询每秒重复触发 LoadLibrary。
pub fn cuda_runtime_status() -> CudaRuntimeStatus {
    let directory = cuda_runtime_candidate_dir();
    let Some(directory) = directory else {
        return CudaRuntimeStatus {
            directory: None,
            error: None,
        };
    };

    if let Ok(mut cache) = CUDA_PREFLIGHT.lock() {
        if let Some(previous) = cache.as_ref() {
            if previous.status.directory.as_ref() == Some(&directory)
                && previous.checked_at.elapsed() < CUDA_PREFLIGHT_CACHE_TTL
            {
                return previous.status.clone();
            }
        }
        let status = match prepare_cuda_runtime(&directory) {
            Ok(()) => CudaRuntimeStatus {
                directory: Some(directory),
                error: None,
            },
            Err(error) => CudaRuntimeStatus {
                directory: Some(directory),
                error: Some(error),
            },
        };
        *cache = Some(CudaPreflightCache {
            checked_at: Instant::now(),
            status: status.clone(),
        });
        return status;
    }

    CudaRuntimeStatus {
        directory: Some(directory.clone()),
        error: Some("CUDA 运行时预检缓存锁不可用".to_string()),
    }
}

pub fn assets_available_for_provider(directory: &Path, provider: ProviderPreference) -> bool {
    if !model_assets_have_expected_sizes(directory) {
        return false;
    }
    runtime_available_for_provider(provider)
}

pub fn runtime_available_for_provider(provider: ProviderPreference) -> bool {
    let cpu_available = cpu_runtime_assets_have_expected_sizes();
    let cuda_available = cuda_runtime_status().available();
    match provider {
        ProviderPreference::Cpu => cpu_available,
        ProviderPreference::Cuda => cuda_available,
        ProviderPreference::Auto => cpu_available || cuda_available,
    }
}

pub fn assets_have_expected_sizes(directory: &Path) -> bool {
    assets_available_for_provider(directory, ProviderPreference::Auto)
}

pub fn snapshot(directory: &Path) -> RuntimeSnapshot {
    let requested_provider = configured_provider();
    let availability = runtime_availability();
    snapshot_with_availability(directory, requested_provider, &availability)
}

#[derive(Debug, Clone)]
struct RuntimeAvailability {
    cpu_runtime_available: bool,
    cuda: CudaRuntimeStatus,
}

impl RuntimeAvailability {
    fn cuda_runtime_available(&self) -> bool {
        self.cuda.available()
    }
}

fn runtime_availability() -> RuntimeAvailability {
    RuntimeAvailability {
        cpu_runtime_available: cpu_runtime_assets_have_expected_sizes(),
        cuda: cuda_runtime_status(),
    }
}

fn snapshot_with_availability(
    directory: &Path,
    requested_provider: ProviderPreference,
    availability: &RuntimeAvailability,
) -> RuntimeSnapshot {
    let cuda_runtime_available = availability.cuda_runtime_available();
    match RUNTIME.lock() {
        Ok(runtime) if runtime.directory.as_deref() == Some(directory) => {
            let provider_changed = runtime.requested_provider != requested_provider;
            RuntimeSnapshot {
                phase: runtime.phase,
                model_dir: directory.to_path_buf(),
                error: runtime.error.clone(),
                requested_provider: requested_provider.as_str().to_string(),
                execution_provider: runtime.execution_provider.as_str().to_string(),
                provider_fallback_reason: if provider_changed {
                    Some("provider 配置已变更；请重启 Sanshu 进程后重新加载模型".to_string())
                } else {
                    runtime.provider_fallback_reason.clone().or_else(|| {
                        fallback_reason_for(
                            requested_provider,
                            cuda_runtime_available,
                            availability.cpu_runtime_available,
                            availability.cuda.error.as_deref(),
                        )
                    })
                },
                cuda_runtime_available,
                cuda_runtime_dir: availability.cuda.directory.clone(),
                cuda_runtime_error: availability.cuda.error.clone(),
                runtime_path: runtime.runtime_path.clone(),
                batch_size: EMBEDDING_BATCH_SIZE,
                intra_threads: runtime.intra_threads,
            }
        }
        Ok(_) => RuntimeSnapshot {
            phase: RuntimePhase::Missing,
            model_dir: directory.to_path_buf(),
            error: None,
            requested_provider: requested_provider.as_str().to_string(),
            execution_provider: ExecutionProvider::Cpu.as_str().to_string(),
            provider_fallback_reason: fallback_reason_for(
                requested_provider,
                cuda_runtime_available,
                availability.cpu_runtime_available,
                availability.cuda.error.as_deref(),
            ),
            cuda_runtime_available,
            cuda_runtime_dir: availability.cuda.directory.clone(),
            cuda_runtime_error: availability.cuda.error.clone(),
            runtime_path: None,
            batch_size: EMBEDDING_BATCH_SIZE,
            intra_threads: None,
        },
        Err(error) => RuntimeSnapshot {
            phase: RuntimePhase::Error,
            model_dir: directory.to_path_buf(),
            error: Some(error.to_string()),
            requested_provider: requested_provider.as_str().to_string(),
            execution_provider: ExecutionProvider::Cpu.as_str().to_string(),
            provider_fallback_reason: fallback_reason_for(
                requested_provider,
                cuda_runtime_available,
                availability.cpu_runtime_available,
                availability.cuda.error.as_deref(),
            ),
            cuda_runtime_available,
            cuda_runtime_dir: availability.cuda.directory.clone(),
            cuda_runtime_error: availability.cuda.error.clone(),
            runtime_path: None,
            batch_size: EMBEDDING_BATCH_SIZE,
            intra_threads: None,
        },
    }
}

fn fallback_reason_for(
    requested_provider: ProviderPreference,
    cuda_runtime_available: bool,
    cpu_runtime_available: bool,
    cuda_runtime_error: Option<&str>,
) -> Option<String> {
    if !requested_provider.allows_cpu_fallback() || cuda_runtime_available || !cpu_runtime_available
    {
        return None;
    }
    Some(match cuda_runtime_error {
        Some(error) => format!("CUDA 运行时预检失败，当前使用 CPU: {}", error),
        None => "CUDA 运行时未找到，当前使用 CPU；可设置 SANSHU_ORT_CUDA_DIR".to_string(),
    })
}

/// 确保进程级 ONNX Runtime 只初始化一次，避免 reranker 先载入 CPU DLL 抢占 CUDA 入口。
pub fn ensure_ort_runtime(
    requested_provider: ProviderPreference,
) -> Result<OrtRuntimeInfo, String> {
    let cuda = cuda_runtime_status();
    let target_cuda = match requested_provider {
        ProviderPreference::Cuda => {
            if !cuda.available() {
                return Err(cuda
                    .error
                    .map(|error| format!("CUDA 运行时预检失败: {}", error))
                    .unwrap_or_else(|| {
                        "CUDA 运行时未找到；可设置 SANSHU_ORT_CUDA_DIR".to_string()
                    }));
            }
            true
        }
        ProviderPreference::Auto => cuda.available(),
        ProviderPreference::Cpu => false,
    };

    let mut state = ORT_RUNTIME
        .lock()
        .map_err(|error| format!("锁定 ONNX Runtime 初始化状态失败: {}", error))?;
    if let Some(info) = state.info.clone() {
        if target_cuda && !info.cuda_capable && requested_provider == ProviderPreference::Cuda {
            return Err("ONNX Runtime 已按 CPU 初始化；切换 CUDA 需要重启 Sanshu 进程".to_string());
        }
        return Ok(info);
    }

    let runtime_path = if target_cuda {
        cuda.directory
            .clone()
            .expect("CUDA 预检成功时必须存在运行时目录")
            .join(ORT_DLL_FILE_NAME)
    } else {
        runtime_dir().join(ORT_DLL_FILE_NAME)
    };
    if !runtime_path.is_file() {
        return Err(format!(
            "ONNX Runtime 核心 DLL 不存在: {}",
            runtime_path.display()
        ));
    }
    let environment = ort::init_from(&runtime_path)
        .map_err(|error| format!("加载 ONNX Runtime {} 失败: {}", ORT_VERSION, error))?;
    if !environment.commit() {
        return Err(
            "ONNX Runtime 已由其他入口初始化，无法确认当前 provider；请重启 Sanshu 进程"
                .to_string(),
        );
    }
    let info = OrtRuntimeInfo {
        runtime_path,
        cuda_capable: target_cuda,
    };
    state.info = Some(info.clone());
    Ok(info)
}

pub fn ensure_started(directory: &Path) {
    let requested_provider = configured_provider();
    let directory = directory.to_path_buf();
    let mut runtime = match RUNTIME.lock() {
        Ok(value) => value,
        Err(error) => {
            log::warn!("[embedding] 锁定共享模型运行时失败: {}", error);
            return;
        }
    };
    if runtime.directory.as_deref() == Some(directory.as_path())
        && runtime.requested_provider == requested_provider
        && matches!(runtime.phase, RuntimePhase::Loading | RuntimePhase::Ready)
    {
        return;
    }
    if !assets_available_for_provider(&directory, requested_provider) {
        runtime.directory = Some(directory);
        runtime.phase = RuntimePhase::Missing;
        runtime.model = None;
        runtime.error = None;
        runtime.requested_provider = requested_provider;
        runtime.execution_provider = ExecutionProvider::Cpu;
        runtime.provider_fallback_reason = None;
        runtime.runtime_path = None;
        runtime.intra_threads = None;
        return;
    }

    runtime.directory = Some(directory.clone());
    runtime.phase = RuntimePhase::Loading;
    runtime.model = None;
    runtime.error = None;
    runtime.requested_provider = requested_provider;
    runtime.execution_provider = ExecutionProvider::Cpu;
    runtime.provider_fallback_reason = None;
    runtime.runtime_path = None;
    runtime.intra_threads = None;
    drop(runtime);

    std::thread::spawn(move || match load_model(&directory, requested_provider) {
        Ok(loaded) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path())
                    && runtime.requested_provider == requested_provider
                {
                    runtime.model = Some(loaded.model);
                    runtime.phase = RuntimePhase::Ready;
                    runtime.error = None;
                    runtime.execution_provider = loaded.execution_provider;
                    runtime.provider_fallback_reason = loaded.provider_fallback_reason;
                    runtime.runtime_path = Some(loaded.runtime_path);
                    runtime.intra_threads = Some(loaded.intra_threads);
                    log::info!(
                        "[embedding] BGE 共享模型运行时已就绪，provider={}，batch={}，intra_threads={}",
                        runtime.execution_provider.as_str(),
                        EMBEDDING_BATCH_SIZE,
                        loaded.intra_threads
                    );
                }
            }
        }
        Err(error) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path())
                    && runtime.requested_provider == requested_provider
                {
                    runtime.phase = RuntimePhase::Error;
                    runtime.error = Some(error.clone());
                }
            }
            log::warn!("[embedding] BGE 共享模型初始化失败: {}", error);
        }
    });
}

pub async fn embed_query(
    directory: &Path,
    query: &str,
    wait_budget: Duration,
) -> Result<Vec<f32>, EmbeddingUnavailable> {
    ensure_started(directory);
    wait_until_ready(directory, wait_budget).await?;
    let directory = directory.to_path_buf();
    let query = format!("{}{}", QUERY_PREFIX, query.trim());
    tokio::task::spawn_blocking(move || embed_texts_ready(&directory, vec![query], 1))
        .await
        .map_err(|error| EmbeddingUnavailable {
            state: "error".to_string(),
            message: format!("等待 BGE 查询向量任务失败: {}", error),
        })?
        .and_then(|mut values| {
            values.pop().ok_or_else(|| EmbeddingUnavailable {
                state: "error".to_string(),
                message: "BGE 未返回查询向量".to_string(),
            })
        })
}

pub fn embed_documents_blocking(
    directory: &Path,
    documents: Vec<String>,
    wait_budget: Duration,
) -> Result<Vec<Vec<f32>>, EmbeddingUnavailable> {
    embed_documents_blocking_with_batch_size(
        directory,
        documents,
        wait_budget,
        EMBEDDING_BATCH_SIZE,
    )
}

pub fn embed_documents_blocking_with_batch_size(
    directory: &Path,
    documents: Vec<String>,
    wait_budget: Duration,
    batch_size: usize,
) -> Result<Vec<Vec<f32>>, EmbeddingUnavailable> {
    ensure_started(directory);
    wait_until_ready_blocking(directory, wait_budget)?;
    embed_texts_ready(directory, documents, batch_size.max(1))
}

pub fn reset() {
    if let Ok(mut runtime) = RUNTIME.lock() {
        *runtime = RuntimeSlot::default();
    }
}

async fn wait_until_ready(
    directory: &Path,
    wait_budget: Duration,
) -> Result<(), EmbeddingUnavailable> {
    let deadline = Instant::now() + wait_budget;
    // 中文说明：等待期间 provider 与 DLL 诊断保持一次快照，避免每 50ms 重新读配置和扫描运行时。
    let requested_provider = configured_provider();
    let availability = runtime_availability();
    loop {
        match snapshot_with_availability(directory, requested_provider, &availability) {
            RuntimeSnapshot {
                phase: RuntimePhase::Ready,
                ..
            } => return Ok(()),
            RuntimeSnapshot {
                phase: RuntimePhase::Missing,
                ..
            } => {
                return Err(EmbeddingUnavailable {
                    state: "missing".to_string(),
                    message: "BGE 模型或 ONNX Runtime 资产尚未就绪".to_string(),
                })
            }
            RuntimeSnapshot {
                phase: RuntimePhase::Error,
                error,
                ..
            } => {
                return Err(EmbeddingUnavailable {
                    state: "error".to_string(),
                    message: error.unwrap_or_else(|| "BGE 运行时初始化失败".to_string()),
                })
            }
            RuntimeSnapshot {
                phase: RuntimePhase::Loading,
                ..
            } if Instant::now() >= deadline => {
                return Err(EmbeddingUnavailable {
                    state: "loading".to_string(),
                    message: "BGE 运行时仍在加载".to_string(),
                })
            }
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

fn wait_until_ready_blocking(
    directory: &Path,
    wait_budget: Duration,
) -> Result<(), EmbeddingUnavailable> {
    let deadline = Instant::now() + wait_budget;
    // 中文说明：阻塞等待同样复用一次 provider/运行时快照，状态变化只读取进程内运行时槽位。
    let requested_provider = configured_provider();
    let availability = runtime_availability();
    loop {
        match snapshot_with_availability(directory, requested_provider, &availability) {
            RuntimeSnapshot {
                phase: RuntimePhase::Ready,
                ..
            } => return Ok(()),
            RuntimeSnapshot {
                phase: RuntimePhase::Missing,
                ..
            } => {
                return Err(EmbeddingUnavailable {
                    state: "missing".to_string(),
                    message: "BGE 模型或 ONNX Runtime 资产尚未就绪".to_string(),
                })
            }
            RuntimeSnapshot {
                phase: RuntimePhase::Error,
                error,
                ..
            } => {
                return Err(EmbeddingUnavailable {
                    state: "error".to_string(),
                    message: error.unwrap_or_else(|| "BGE 运行时初始化失败".to_string()),
                })
            }
            RuntimeSnapshot {
                phase: RuntimePhase::Loading,
                ..
            } if Instant::now() >= deadline => {
                return Err(EmbeddingUnavailable {
                    state: "loading".to_string(),
                    message: "BGE 运行时仍在加载".to_string(),
                })
            }
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn embed_texts_ready(
    directory: &Path,
    texts: Vec<String>,
    batch_size: usize,
) -> Result<Vec<Vec<f32>>, EmbeddingUnavailable> {
    let mut runtime = RUNTIME.lock().map_err(|error| EmbeddingUnavailable {
        state: "error".to_string(),
        message: format!("锁定 BGE 共享模型失败: {}", error),
    })?;
    if runtime.directory.as_deref() != Some(directory) || runtime.phase != RuntimePhase::Ready {
        return Err(EmbeddingUnavailable {
            state: "loading".to_string(),
            message: "BGE 共享模型运行时尚未就绪".to_string(),
        });
    }
    let model = runtime.model.as_mut().ok_or_else(|| EmbeddingUnavailable {
        state: "error".to_string(),
        message: "BGE 共享模型实例缺失".to_string(),
    })?;
    let inputs = texts.iter().map(String::as_str).collect::<Vec<_>>();
    let embeddings =
        model
            .embed(inputs, Some(batch_size))
            .map_err(|error| EmbeddingUnavailable {
                state: "error".to_string(),
                message: format!("生成 BGE 向量失败: {}", error),
            })?;
    if embeddings
        .iter()
        .any(|embedding| embedding.len() != MODEL_DIMENSION)
    {
        return Err(EmbeddingUnavailable {
            state: "error".to_string(),
            message: format!("BGE 向量维度与预期 {} 不一致", MODEL_DIMENSION),
        });
    }
    Ok(embeddings)
}

fn load_model(
    directory: &Path,
    requested_provider: ProviderPreference,
) -> Result<LoadedModel, String> {
    let mut fallback_reason = None;
    if requested_provider.attempts_cuda() {
        match ensure_ort_runtime(ProviderPreference::Cuda) {
            Ok(runtime) => {
                match create_embedding_model(directory, ExecutionProvider::Cuda, &runtime) {
                    Ok((model, intra_threads)) => {
                        return Ok(LoadedModel {
                            model,
                            execution_provider: ExecutionProvider::Cuda,
                            provider_fallback_reason: None,
                            runtime_path: runtime.runtime_path,
                            intra_threads,
                        });
                    }
                    Err(error) if requested_provider.allows_cpu_fallback() => {
                        // 中文说明：CUDA 核心已载入时不能再次切换 DLL；在同一核心上创建 CPU 会话完成回退。
                        fallback_reason = Some(format!("CUDA 初始化失败，已回退 CPU: {}", error));
                        log::warn!(
                            "[embedding] CUDA provider 初始化失败，将在同一 ORT 核心回退 CPU: {}",
                            error
                        );
                        let (model, intra_threads) =
                            create_embedding_model(directory, ExecutionProvider::Cpu, &runtime)?;
                        return Ok(LoadedModel {
                            model,
                            execution_provider: ExecutionProvider::Cpu,
                            provider_fallback_reason: fallback_reason,
                            runtime_path: runtime.runtime_path,
                            intra_threads,
                        });
                    }
                    Err(error) => return Err(format!("CUDA 初始化失败: {}", error)),
                }
            }
            Err(error) if requested_provider.allows_cpu_fallback() => {
                fallback_reason =
                    Some(format!("CUDA 运行时预检/初始化失败，已回退 CPU: {}", error));
                log::warn!("[embedding] CUDA 运行时不可用，将回退 CPU: {}", error);
            }
            Err(error) => return Err(format!("CUDA 初始化失败: {}", error)),
        }
    }

    let runtime = ensure_ort_runtime(ProviderPreference::Cpu).map_err(|error| {
        fallback_reason
            .as_ref()
            .map(|reason| format!("{}；CPU 初始化失败: {}", reason, error))
            .unwrap_or(error)
    })?;
    let (model, intra_threads) =
        create_embedding_model(directory, ExecutionProvider::Cpu, &runtime).map_err(|error| {
            fallback_reason
                .as_ref()
                .map(|reason| format!("{}；CPU 初始化失败: {}", reason, error))
                .unwrap_or(error)
        })?;
    Ok(LoadedModel {
        model,
        execution_provider: ExecutionProvider::Cpu,
        provider_fallback_reason: fallback_reason,
        runtime_path: runtime.runtime_path,
        intra_threads,
    })
}

fn create_embedding_model(
    directory: &Path,
    provider: ExecutionProvider,
    runtime: &OrtRuntimeInfo,
) -> Result<(TextEmbedding, usize), String> {
    if provider == ExecutionProvider::Cuda && !runtime.cuda_capable {
        return Err("当前 ONNX Runtime 核心不含可用 CUDA provider".to_string());
    }

    let threads = match provider {
        ExecutionProvider::Cpu => std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1),
        ExecutionProvider::Cuda => CUDA_INTRA_THREADS,
    };
    let options = InitOptionsUserDefined::new()
        .with_max_length(512)
        .with_intra_threads(threads);
    #[cfg(feature = "cuda")]
    let options = if provider == ExecutionProvider::Cuda {
        options
            .with_execution_providers(vec![ort::ep::CUDA::default().build().error_on_failure()])
            .with_disable_cpu_fallback(true)
    } else {
        options
    };
    #[cfg(not(feature = "cuda"))]
    if provider == ExecutionProvider::Cuda {
        return Err("当前构建未启用 ORT CUDA provider".to_string());
    }

    let model = user_defined_model(directory)?;
    let embedding = TextEmbedding::try_new_from_user_defined(model, options)
        .map_err(|error| format!("创建 BGE ONNX 会话失败: {}", error))?;
    Ok((embedding, threads))
}

#[cfg(feature = "cuda")]
fn prepare_cuda_runtime(directory: &Path) -> Result<(), String> {
    ort::util::preload_dylib(directory.join(CUDA_PROVIDER_DLL_FILE_NAME))
        .map_err(|error| format!("加载 CUDA provider DLL 失败: {}", error))
}

#[cfg(not(feature = "cuda"))]
fn prepare_cuda_runtime(_directory: &Path) -> Result<(), String> {
    Err("当前构建未启用 ORT CUDA provider".to_string())
}

fn user_defined_model(directory: &Path) -> Result<UserDefinedEmbeddingModel, String> {
    Ok(UserDefinedEmbeddingModel::new(
        read_file(&directory.join("onnx/model.onnx"))?,
        TokenizerFiles {
            tokenizer_file: read_file(&directory.join("tokenizer.json"))?,
            config_file: read_file(&directory.join("config.json"))?,
            special_tokens_map_file: read_file(&directory.join("special_tokens_map.json"))?,
            tokenizer_config_file: read_file(&directory.join("tokenizer_config.json"))?,
        },
    )
    .with_pooling(Pooling::Cls))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("读取模型文件 {} 失败: {}", path.display(), error))
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_directory_prefers_new_setting_then_legacy_setting() {
        assert_eq!(
            effective_model_dir(Some("D:/shared"), Some("D:/legacy")),
            PathBuf::from("D:/shared")
        );
        assert_eq!(
            effective_model_dir(None, Some("D:/legacy")),
            PathBuf::from("D:/legacy")
        );
    }

    #[test]
    fn cosine_similarity_handles_equal_and_orthogonal_vectors() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.0001);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 0.0001);
    }
}
