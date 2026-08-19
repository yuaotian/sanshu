//! ACE 索引任务清单与任务事件。
//!
//! `projects.json` 只保存已经可用于检索的 blob 名称，不能作为上传中的断点。
//! 本模块单独维护 `index_jobs.json`，每个成功批次都会写入检查点，进程重启后
//! 可以据此重新收集文件并跳过已经确认成功的 blob。

use anyhow::Result;
use chrono::{DateTime, Utc};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub(crate) const JOB_QUEUED: &str = "queued";
pub(crate) const JOB_COLLECTING: &str = "collecting";
pub(crate) const JOB_UPLOADING: &str = "uploading";
pub(crate) const JOB_PAUSED: &str = "paused";
pub(crate) const JOB_SCOPE_BLOCKED: &str = "scope_blocked";
pub(crate) const JOB_COMPLETED: &str = "completed";
pub(crate) const JOB_FAILED: &str = "failed";
pub(crate) const INDEX_JOB_EVENT: &str = "acemcp-index-job";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IndexJobEvent {
    pub event_id: String,
    pub job_id: String,
    pub project_root: String,
    pub event_type: String,
    pub status: String,
    pub progress: u8,
    pub total_blobs: usize,
    pub completed_blobs: usize,
    pub total_batches: usize,
    pub completed_batches: usize,
    pub message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IndexJob {
    pub job_id: String,
    pub project_root: String,
    /// incremental | full
    pub mode: String,
    /// queued | collecting | uploading | paused | scope_blocked | completed | failed
    pub status: String,
    /// ACE 连接身份与索引参数的综合签名。
    pub scope_hash: String,
    pub config_fingerprint: String,
    pub total_blobs: usize,
    pub completed_blobs: usize,
    pub total_batches: usize,
    pub completed_batches: usize,
    /// 已经收到 ACE 成功响应的本地 blob 哈希。
    #[serde(default)]
    pub completed_blob_hashes: Vec<String>,
    /// ACE 返回的远端 blob 名称，允许每批成功后立即写入 projects.json。
    #[serde(default)]
    pub uploaded_blob_names: Vec<String>,
    #[serde(default)]
    pub failed_batches: Vec<usize>,
    /// 当前执行期间又收到索引请求时，记录完成后需要接续的任务模式。
    #[serde(default)]
    pub rerun_mode: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub events: Vec<IndexJobEvent>,
}

impl IndexJob {
    pub(crate) fn new(
        project_root: String,
        mode: impl Into<String>,
        scope_hash: String,
        config_fingerprint: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            job_id: Uuid::new_v4().to_string(),
            project_root,
            mode: mode.into(),
            status: JOB_QUEUED.to_string(),
            scope_hash,
            config_fingerprint,
            total_blobs: 0,
            completed_blobs: 0,
            total_batches: 0,
            completed_batches: 0,
            completed_blob_hashes: Vec::new(),
            uploaded_blob_names: Vec::new(),
            failed_batches: Vec::new(),
            rerun_mode: None,
            last_error: None,
            created_at: now.clone(),
            updated_at: now,
            events: Vec::new(),
        }
    }

    pub(crate) fn is_resumable(&self) -> bool {
        matches!(
            self.status.as_str(),
            JOB_QUEUED | JOB_COLLECTING | JOB_UPLOADING | JOB_PAUSED
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct IndexJobsManifest {
    pub jobs: HashMap<String, IndexJob>,
}

static MANIFEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static EVENT_APP: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();

/// 持有项目级操作系统锁直到 worker 结束；进程崩溃时由操作系统自动释放。
pub(crate) struct ProjectLease {
    _file: File,
}

fn manifest_lock() -> &'static Mutex<()> {
    MANIFEST_LOCK.get_or_init(|| Mutex::new(()))
}

fn event_app() -> &'static Mutex<Option<AppHandle>> {
    EVENT_APP.get_or_init(|| Mutex::new(None))
}

/// 注册 GUI 事件出口。MCP 独立进程没有 AppHandle，但仍会把事件写入 manifest。
pub(crate) fn register_event_app(app: &AppHandle) {
    if let Ok(mut current) = event_app().lock() {
        *current = Some(app.clone());
    }
}

pub(crate) fn home_index_jobs_file() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = home.join(".acemcp").join("data");
    let _ = fs::create_dir_all(&data_dir);
    data_dir.join("index_jobs.json")
}

fn manifest_file_lock_path() -> PathBuf {
    home_index_jobs_file().with_extension("lock")
}

fn acquire_manifest_file_lock() -> Result<File> {
    let path = manifest_file_lock_path();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)?;
    file.lock()
        .map_err(|error| anyhow::anyhow!("锁定 ACE 索引任务清单失败: {}", error))?;
    Ok(file)
}

fn load_manifest_unlocked() -> IndexJobsManifest {
    let path = home_index_jobs_file();
    let backup_path = path.with_extension("json.bak");
    for candidate in [&path, &backup_path] {
        let Ok(data) = fs::read_to_string(candidate) else {
            continue;
        };
        match serde_json::from_str(&data) {
            Ok(manifest) => return manifest,
            Err(error) => {
                log::warn!(
                    "读取 ACE 索引任务清单失败: path={}, error={}",
                    candidate.display(),
                    error
                );
            }
        }
    }
    IndexJobsManifest::default()
}

/// 读取任务清单时也持有跨进程锁，避免读到原子替换过程中的中间状态。
pub(crate) fn load_manifest() -> IndexJobsManifest {
    match acquire_manifest_file_lock() {
        Ok(_file_lock) => load_manifest_unlocked(),
        Err(error) => {
            // 读取接口没有 Result 契约；锁异常时保留可用的只读降级，并记录原因。
            log::warn!("读取 ACE 索引任务清单时无法获取文件锁: {}", error);
            load_manifest_unlocked()
        }
    }
}

fn atomic_write(path: &PathBuf, data: &str) -> Result<()> {
    let tmp_path = path.with_extension(format!("json.tmp-{}", Uuid::new_v4()));
    let backup_path = path.with_extension("json.bak");
    fs::write(&tmp_path, data)?;
    // Windows 目标存在时可能拒绝直接 rename，先保留可恢复备份，避免突然退出丢失断点。
    let had_original = path.exists();
    if had_original {
        let _ = fs::remove_file(&backup_path);
        if let Err(error) = fs::rename(path, &backup_path) {
            log::warn!("备份 ACE 索引任务清单失败，将退回直接写入: {}", error);
            fs::write(path, data)?;
            let _ = fs::remove_file(&tmp_path);
            return Ok(());
        }
    }
    if let Err(rename_error) = fs::rename(&tmp_path, path) {
        log::warn!("原子替换 ACE 索引任务清单失败，将尝试恢复备份: {}", rename_error);
        if had_original && backup_path.exists() {
            let _ = fs::rename(&backup_path, path);
        }
        fs::write(path, data)?;
        let _ = fs::remove_file(&tmp_path);
    } else if had_original {
        let _ = fs::remove_file(&backup_path);
    }
    Ok(())
}

fn save_manifest_unlocked(manifest: &IndexJobsManifest) -> Result<()> {
    let path = home_index_jobs_file();
    let data = serde_json::to_string_pretty(manifest)?;
    atomic_write(&path, &data)
}

pub(crate) fn get_job(project_root: &str) -> Option<IndexJob> {
    let _guard = manifest_lock().lock().ok()?;
    let _file_lock = acquire_manifest_file_lock().ok();
    load_manifest_unlocked().jobs.get(project_root).cloned()
}

pub(crate) fn resumable_jobs() -> Vec<IndexJob> {
    let _guard = match manifest_lock().lock() {
        Ok(guard) => guard,
        Err(_) => return Vec::new(),
    };
    let _file_lock = acquire_manifest_file_lock().ok();
    load_manifest_unlocked()
        .jobs
        .into_values()
        .filter(IndexJob::is_resumable)
        .collect()
}

pub(crate) fn create_job(
    project_root: &str,
    mode: &str,
    scope_hash: &str,
    config_fingerprint: &str,
) -> Result<IndexJob> {
    let _guard = manifest_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("获取 ACE 索引任务清单锁失败"))?;
    let _file_lock = acquire_manifest_file_lock()?;
    let mut manifest = load_manifest_unlocked();
    let job = IndexJob::new(
        project_root.to_string(),
        mode,
        scope_hash.to_string(),
        config_fingerprint.to_string(),
    );
    manifest.jobs.insert(project_root.to_string(), job.clone());
    save_manifest_unlocked(&manifest)?;
    Ok(job)
}

/// 更新任务并持久化事件；事件同时写入 manifest，GUI 有 AppHandle 时再广播。
pub(crate) fn update_job<F>(
    project_root: &str,
    event_type: &str,
    message: Option<String>,
    updater: F,
) -> Result<Option<IndexJob>>
where
    F: FnOnce(&mut IndexJob),
{
    let updated = {
        let _guard = manifest_lock()
            .lock()
            .map_err(|_| anyhow::anyhow!("获取 ACE 索引任务清单锁失败"))?;
        let _file_lock = acquire_manifest_file_lock()?;
        let mut manifest = load_manifest_unlocked();
        let Some(job) = manifest.jobs.get_mut(project_root) else {
            return Ok(None);
        };
        updater(job);
        job.updated_at = Utc::now();
        let event = IndexJobEvent {
            event_id: Uuid::new_v4().to_string(),
            job_id: job.job_id.clone(),
            project_root: job.project_root.clone(),
            event_type: event_type.to_string(),
            status: job.status.clone(),
            progress: if job.total_blobs == 0 {
                0
            } else {
                ((job.completed_blobs.saturating_mul(100) / job.total_blobs).min(100)) as u8
            },
            total_blobs: job.total_blobs,
            completed_blobs: job.completed_blobs,
            total_batches: job.total_batches,
            completed_batches: job.completed_batches,
            message,
            updated_at: job.updated_at.clone(),
        };
        job.events.push(event);
        if job.events.len() > 100 {
            let drop_count = job.events.len() - 100;
            job.events.drain(0..drop_count);
        }
        let updated = job.clone();
        save_manifest_unlocked(&manifest)?;
        updated
    };

    publish_latest_event(&updated);
    Ok(Some(updated))
}

pub(crate) fn remove_job(project_root: &str) -> Result<bool> {
    let _guard = manifest_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("获取 ACE 索引任务清单锁失败"))?;
    let _file_lock = acquire_manifest_file_lock()?;
    let mut manifest = load_manifest_unlocked();
    let removed = manifest.jobs.remove(project_root).is_some();
    if removed {
        save_manifest_unlocked(&manifest)?;
    }
    Ok(removed)
}

fn project_lease_path(project_root: &str) -> PathBuf {
    let jobs_path = home_index_jobs_file();
    let lease_dir = jobs_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("index_job_leases");
    let digest = Md5::digest(project_root.as_bytes());
    lease_dir.join(format!("{}.lock", hex::encode(digest)))
}

/// 尝试获取项目级 worker lease；`None` 表示另一个进程正在上传。
pub(crate) fn try_acquire_project_lease(project_root: &str) -> Result<Option<ProjectLease>> {
    let path = project_lease_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(ProjectLease { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(anyhow::anyhow!(
            "获取 ACE 项目 worker lease 失败: path={}, error={}",
            path.display(),
            error
        )),
    }
}

fn publish_latest_event(job: &IndexJob) {
    let Some(event) = job.events.last().cloned() else {
        return;
    };
    let app = event_app().lock().ok().and_then(|guard| guard.clone());
    if let Some(app) = app {
        let _ = app.emit(INDEX_JOB_EVENT, &event);
    }
}
