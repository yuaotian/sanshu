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

const MODEL_FILES: &[(&str, u64)] = &[
    ("onnx/model.onnx", 94_851_877),
    ("config.json", 716),
    ("special_tokens_map.json", 125),
    ("tokenizer.json", 439_125),
    ("tokenizer_config.json", 367),
];
const ORT_DLL_FILE_NAME: &str = "onnxruntime.dll";
const ORT_DLL_BYTES: u64 = 15_809_848;

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
}

#[derive(Debug, Clone)]
pub struct EmbeddingUnavailable {
    pub state: String,
    pub message: String,
}

struct RuntimeSlot {
    directory: Option<PathBuf>,
    phase: RuntimePhase,
    model: Option<TextEmbedding>,
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

pub fn assets_have_expected_sizes(directory: &Path) -> bool {
    MODEL_FILES.iter().all(|(relative_path, size)| {
        fs::metadata(directory.join(relative_path))
            .map(|metadata| metadata.is_file() && metadata.len() == *size)
            .unwrap_or(false)
    }) && fs::metadata(runtime_dir().join(ORT_DLL_FILE_NAME))
        .map(|metadata| metadata.is_file() && metadata.len() == ORT_DLL_BYTES)
        .unwrap_or(false)
}

pub fn snapshot(directory: &Path) -> RuntimeSnapshot {
    match RUNTIME.lock() {
        Ok(runtime) if runtime.directory.as_deref() == Some(directory) => RuntimeSnapshot {
            phase: runtime.phase,
            model_dir: directory.to_path_buf(),
            error: runtime.error.clone(),
        },
        Ok(_) => RuntimeSnapshot {
            phase: RuntimePhase::Missing,
            model_dir: directory.to_path_buf(),
            error: None,
        },
        Err(error) => RuntimeSnapshot {
            phase: RuntimePhase::Error,
            model_dir: directory.to_path_buf(),
            error: Some(error.to_string()),
        },
    }
}

pub fn ensure_started(directory: &Path) {
    let directory = directory.to_path_buf();
    let mut runtime = match RUNTIME.lock() {
        Ok(value) => value,
        Err(error) => {
            log::warn!("[embedding] 锁定共享模型运行时失败: {}", error);
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
            log::info!("[embedding] BGE 共享模型运行时已就绪");
        }
        Err(error) => {
            if let Ok(mut runtime) = RUNTIME.lock() {
                if runtime.directory.as_deref() == Some(directory.as_path()) {
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
    ensure_started(directory);
    wait_until_ready_blocking(directory, wait_budget)?;
    embed_texts_ready(directory, documents, 32)
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
    loop {
        match snapshot(directory) {
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
    loop {
        match snapshot(directory) {
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

fn load_model(directory: &Path) -> Result<TextEmbedding, String> {
    ort::init_from(runtime_dir().join(ORT_DLL_FILE_NAME))
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
    TextEmbedding::try_new_from_user_defined(
        model,
        InitOptionsUserDefined::new().with_max_length(512),
    )
    .map_err(|error| format!("创建 BGE ONNX 会话失败: {}", error))
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
