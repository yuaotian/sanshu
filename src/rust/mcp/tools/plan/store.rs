use once_cell::sync::Lazy;
use ring::digest::{Context, SHA256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use thiserror::Error;

use super::types::{
    PlanAction, PlanFile, PlanItem, PlanRequest, PlanResult, PlanStatus, PLAN_FILE_VERSION,
};

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("计划参数无效：{0}")]
    Validation(String),
    #[error("计划状态冲突：{0}")]
    Conflict(String),
    #[error("计划文件损坏：{0}")]
    Corrupt(String),
    #[error("计划存储失败：{0}")]
    Storage(String),
}

pub struct PlanStore {
    root_dir: PathBuf,
    workspace_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl PlanStore {
    pub fn from_system_config() -> Result<Self, PlanError> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| PlanError::Storage("无法确定系统配置目录".to_string()))?;
        Ok(Self::new(config_dir.join("sanshu").join("plans")))
    }

    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            workspace_locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn ensure_root_dir(&self) -> Result<(), PlanError> {
        fs::create_dir_all(&self.root_dir).map_err(|error| {
            PlanError::Storage(format!(
                "创建计划目录 {} 失败：{}",
                self.root_dir.display(),
                error
            ))
        })
    }

    pub fn execute(&self, request: PlanRequest) -> Result<PlanResult, PlanError> {
        let workspace = Self::normalize_workspace(&request.workspace)?;
        Self::validate_action_fields(&request)?;

        let workspace_lock = self.workspace_lock(&workspace)?;
        let _guard = workspace_lock
            .lock()
            .map_err(|_| PlanError::Storage("工作区计划锁已损坏".to_string()))?;

        match request.action {
            PlanAction::Set => self.set_plan(workspace, request.items.unwrap_or_default()),
            PlanAction::Update => self.update_plan(
                workspace,
                request.id.unwrap_or_default(),
                request.status.expect("update 已校验 status"),
            ),
            PlanAction::Get => self.get_plan_locked(workspace),
            PlanAction::Clear => self.clear_plan(workspace),
        }
    }

    pub fn get_snapshot(&self, workspace: &str) -> Result<PlanResult, PlanError> {
        self.execute(PlanRequest {
            action: PlanAction::Get,
            workspace: workspace.to_string(),
            items: None,
            id: None,
            status: None,
        })
    }

    pub fn plan_file_path(&self, workspace: &str) -> Result<PathBuf, PlanError> {
        let normalized = Self::normalize_workspace(workspace)?;
        Ok(self
            .root_dir
            .join(format!("{}.json", Self::workspace_hash(&normalized))))
    }

    pub fn normalize_workspace(workspace: &str) -> Result<String, PlanError> {
        let trimmed = workspace.trim();
        if trimmed.is_empty() {
            return Err(PlanError::Validation("workspace 不能为空".to_string()));
        }

        let path = PathBuf::from(trimmed);
        if !path.is_absolute() {
            return Err(PlanError::Validation(
                "workspace 必须是绝对路径".to_string(),
            ));
        }

        let normalized_path = fs::canonicalize(&path).unwrap_or(path);
        let mut normalized = normalized_path.to_string_lossy().replace('\\', "/");

        // 中文说明：canonicalize 在 Windows 上可能返回扩展路径前缀，哈希前统一移除。
        if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
            normalized = format!("//{}", rest);
        } else if let Some(rest) = normalized.strip_prefix("//?/") {
            normalized = rest.to_string();
        }

        while normalized.ends_with('/') && normalized.len() > 3 {
            normalized.pop();
        }

        #[cfg(windows)]
        {
            normalized = normalized.to_lowercase();
        }

        Ok(normalized)
    }

    fn workspace_hash(workspace: &str) -> String {
        let mut context = Context::new(&SHA256);
        context.update(workspace.as_bytes());
        hex::encode(context.finish().as_ref())
    }

    fn workspace_lock(&self, workspace: &str) -> Result<Arc<Mutex<()>>, PlanError> {
        let mut locks = self
            .workspace_locks
            .lock()
            .map_err(|_| PlanError::Storage("计划锁表已损坏".to_string()))?;
        Ok(locks
            .entry(workspace.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }

    fn validate_action_fields(request: &PlanRequest) -> Result<(), PlanError> {
        match request.action {
            PlanAction::Set => {
                if request.id.is_some() || request.status.is_some() {
                    return Err(PlanError::Validation(
                        "set 只允许 workspace 和 items".to_string(),
                    ));
                }
                let items = request
                    .items
                    .as_ref()
                    .ok_or_else(|| PlanError::Validation("set 必须提供 items".to_string()))?;
                if items.is_empty() {
                    return Err(PlanError::Validation(
                        "set 的 items 不能为空，请使用 clear 清空计划".to_string(),
                    ));
                }
            }
            PlanAction::Update => {
                if request.items.is_some() {
                    return Err(PlanError::Validation("update 不允许提供 items".to_string()));
                }
                if request.id.as_deref().is_none_or(|id| id.trim().is_empty()) {
                    return Err(PlanError::Validation("update 必须提供非空 id".to_string()));
                }
                if request.status.is_none() {
                    return Err(PlanError::Validation("update 必须提供 status".to_string()));
                }
            }
            PlanAction::Get | PlanAction::Clear => {
                if request.items.is_some() || request.id.is_some() || request.status.is_some() {
                    return Err(PlanError::Validation(format!(
                        "{} 只允许提供 workspace",
                        request.action.as_str()
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_items(items: &mut [PlanItem]) -> Result<(), PlanError> {
        let mut ids = HashSet::new();
        let mut in_progress_count = 0usize;

        for item in items {
            item.id = item.id.trim().to_string();
            item.text = item.text.trim().to_string();
            if item.id.is_empty() {
                return Err(PlanError::Validation("计划项 id 不能为空".to_string()));
            }
            if item.text.is_empty() {
                return Err(PlanError::Validation(format!(
                    "计划项 {} 的文本不能为空",
                    item.id
                )));
            }
            if !ids.insert(item.id.clone()) {
                return Err(PlanError::Validation(format!(
                    "计划项 id 重复：{}",
                    item.id
                )));
            }
            if item.status == PlanStatus::InProgress {
                in_progress_count += 1;
            }
        }

        if in_progress_count > 1 {
            return Err(PlanError::Validation(
                "同一计划最多只能有一个 in_progress 项".to_string(),
            ));
        }
        Ok(())
    }

    fn set_plan(
        &self,
        workspace: String,
        mut items: Vec<PlanItem>,
    ) -> Result<PlanResult, PlanError> {
        Self::validate_items(&mut items)?;
        let new_file = PlanFile {
            version: PLAN_FILE_VERSION,
            workspace: workspace.clone(),
            items: items.clone(),
        };

        let path = self.path_for_normalized_workspace(&workspace);
        let changed = match self.read_file(&path, &workspace) {
            Ok(Some(current)) => current != new_file,
            Ok(None) | Err(PlanError::Corrupt(_)) => true,
            Err(error) => return Err(error),
        };

        if changed {
            self.write_file(&path, &new_file)?;
        }

        Ok(PlanResult::new(PlanAction::Set, workspace, changed, items))
    }

    fn update_plan(
        &self,
        workspace: String,
        id: String,
        status: PlanStatus,
    ) -> Result<PlanResult, PlanError> {
        let id = id.trim().to_string();
        let path = self.path_for_normalized_workspace(&workspace);
        let mut file = self
            .read_file(&path, &workspace)?
            .ok_or_else(|| PlanError::Conflict("当前工作区没有计划，请先调用 set".to_string()))?;

        let index = file
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| PlanError::Conflict(format!("计划项不存在：{}", id)))?;
        let current = file.items[index].status;

        if current == status {
            return Ok(PlanResult::new(
                PlanAction::Update,
                workspace,
                false,
                file.items,
            ));
        }

        let valid_transition = matches!(
            (current, status),
            (PlanStatus::Pending, PlanStatus::InProgress)
                | (PlanStatus::InProgress, PlanStatus::Completed)
        );
        if !valid_transition {
            return Err(PlanError::Conflict(format!(
                "计划项 {} 不允许从 {} 变更为 {}",
                id,
                current.as_str(),
                status.as_str()
            )));
        }

        if status == PlanStatus::InProgress
            && file.items.iter().enumerate().any(|(item_index, item)| {
                item_index != index && item.status == PlanStatus::InProgress
            })
        {
            return Err(PlanError::Conflict(
                "已有其他计划项处于 in_progress，请先完成该项".to_string(),
            ));
        }

        file.items[index].status = status;
        self.write_file(&path, &file)?;
        Ok(PlanResult::new(
            PlanAction::Update,
            workspace,
            true,
            file.items,
        ))
    }

    fn get_plan_locked(&self, workspace: String) -> Result<PlanResult, PlanError> {
        let path = self.path_for_normalized_workspace(&workspace);
        let items = self
            .read_file(&path, &workspace)?
            .unwrap_or_else(|| PlanFile::empty(workspace.clone()))
            .items;
        Ok(PlanResult::new(PlanAction::Get, workspace, false, items))
    }

    fn clear_plan(&self, workspace: String) -> Result<PlanResult, PlanError> {
        let path = self.path_for_normalized_workspace(&workspace);
        let changed = if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                PlanError::Storage(format!("删除计划文件 {} 失败：{}", path.display(), error))
            })?;
            true
        } else {
            false
        };
        Ok(PlanResult::new(
            PlanAction::Clear,
            workspace,
            changed,
            Vec::new(),
        ))
    }

    fn path_for_normalized_workspace(&self, workspace: &str) -> PathBuf {
        self.root_dir
            .join(format!("{}.json", Self::workspace_hash(workspace)))
    }

    fn read_file(&self, path: &Path, workspace: &str) -> Result<Option<PlanFile>, PlanError> {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read(path).map_err(|error| {
            PlanError::Storage(format!("读取计划文件 {} 失败：{}", path.display(), error))
        })?;
        let mut file: PlanFile = serde_json::from_slice(&content).map_err(|error| {
            PlanError::Corrupt(format!("{} 无法解析：{}", path.display(), error))
        })?;

        if file.version != PLAN_FILE_VERSION {
            return Err(PlanError::Corrupt(format!(
                "{} 的版本 {} 不受支持",
                path.display(),
                file.version
            )));
        }
        if file.workspace != workspace {
            return Err(PlanError::Corrupt(format!(
                "{} 的 workspace 与请求不匹配",
                path.display()
            )));
        }
        Self::validate_items(&mut file.items).map_err(|error| {
            PlanError::Corrupt(format!("{} 的计划项无效：{}", path.display(), error))
        })?;
        Ok(Some(file))
    }

    fn write_file(&self, path: &Path, file: &PlanFile) -> Result<(), PlanError> {
        self.ensure_root_dir()?;
        let content = serde_json::to_vec_pretty(file)
            .map_err(|error| PlanError::Storage(format!("序列化计划失败：{}", error)))?;

        let mut temp_file = NamedTempFile::new_in(&self.root_dir)
            .map_err(|error| PlanError::Storage(format!("创建计划临时文件失败：{}", error)))?;
        temp_file
            .write_all(&content)
            .map_err(|error| PlanError::Storage(format!("写入计划临时文件失败：{}", error)))?;
        temp_file
            .flush()
            .map_err(|error| PlanError::Storage(format!("刷新计划临时文件失败：{}", error)))?;
        temp_file
            .as_file()
            .sync_all()
            .map_err(|error| PlanError::Storage(format!("同步计划临时文件失败：{}", error)))?;
        temp_file.persist(path).map_err(|error| {
            PlanError::Storage(format!(
                "原子替换计划文件 {} 失败：{}",
                path.display(),
                error
            ))
        })?;
        Ok(())
    }
}

static PLAN_STORE: Lazy<Result<PlanStore, String>> =
    Lazy::new(|| PlanStore::from_system_config().map_err(|error| error.to_string()));

pub fn get_plan_store() -> Result<&'static PlanStore, PlanError> {
    PLAN_STORE
        .as_ref()
        .map_err(|error| PlanError::Storage(error.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    fn request(action: PlanAction, workspace: &Path) -> PlanRequest {
        PlanRequest {
            action,
            workspace: workspace.to_string_lossy().to_string(),
            items: None,
            id: None,
            status: None,
        }
    }

    fn items() -> Vec<PlanItem> {
        vec![
            PlanItem {
                id: "step-1".to_string(),
                text: "实现存储".to_string(),
                status: PlanStatus::Pending,
            },
            PlanItem {
                id: "step-2".to_string(),
                text: "实现界面".to_string(),
                status: PlanStatus::Pending,
            },
        ]
    }

    #[test]
    fn set_get_update_clear_are_consistent_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = PlanStore::new(temp.path().join("plans"));

        let mut set = request(PlanAction::Set, &workspace);
        set.items = Some(items());
        assert!(store.execute(set.clone()).unwrap().changed);
        assert!(!store.execute(set).unwrap().changed);

        let mut start = request(PlanAction::Update, &workspace);
        start.id = Some("step-1".to_string());
        start.status = Some(PlanStatus::InProgress);
        assert!(store.execute(start.clone()).unwrap().changed);
        assert!(!store.execute(start).unwrap().changed);

        let mut complete = request(PlanAction::Update, &workspace);
        complete.id = Some("step-1".to_string());
        complete.status = Some(PlanStatus::Completed);
        let completed = store.execute(complete.clone()).unwrap();
        assert!(completed.changed);
        assert_eq!(completed.summary.completed, 1);
        assert!(!store.execute(complete).unwrap().changed);

        let snapshot = store.execute(request(PlanAction::Get, &workspace)).unwrap();
        assert_eq!(snapshot.items[0].status, PlanStatus::Completed);

        assert!(
            store
                .execute(request(PlanAction::Clear, &workspace))
                .unwrap()
                .changed
        );
        assert!(
            !store
                .execute(request(PlanAction::Clear, &workspace))
                .unwrap()
                .changed
        );
    }

    #[test]
    fn invalid_transitions_and_second_active_item_do_not_change_plan() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = PlanStore::new(temp.path().join("plans"));
        let mut set = request(PlanAction::Set, &workspace);
        set.items = Some(items());
        store.execute(set).unwrap();

        let mut skip = request(PlanAction::Update, &workspace);
        skip.id = Some("step-1".to_string());
        skip.status = Some(PlanStatus::Completed);
        assert!(matches!(store.execute(skip), Err(PlanError::Conflict(_))));

        let mut start_first = request(PlanAction::Update, &workspace);
        start_first.id = Some("step-1".to_string());
        start_first.status = Some(PlanStatus::InProgress);
        store.execute(start_first).unwrap();

        let mut start_second = request(PlanAction::Update, &workspace);
        start_second.id = Some("step-2".to_string());
        start_second.status = Some(PlanStatus::InProgress);
        assert!(matches!(
            store.execute(start_second),
            Err(PlanError::Conflict(_))
        ));

        let snapshot = store.execute(request(PlanAction::Get, &workspace)).unwrap();
        assert_eq!(snapshot.items[0].status, PlanStatus::InProgress);
        assert_eq!(snapshot.items[1].status, PlanStatus::Pending);
    }

    #[test]
    fn workspaces_are_isolated_and_corrupt_files_require_explicit_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_a = temp.path().join("a");
        let workspace_b = temp.path().join("b");
        fs::create_dir_all(&workspace_a).unwrap();
        fs::create_dir_all(&workspace_b).unwrap();
        let store = PlanStore::new(temp.path().join("plans"));

        let mut set_a = request(PlanAction::Set, &workspace_a);
        set_a.items = Some(items());
        store.execute(set_a).unwrap();
        assert!(store
            .execute(request(PlanAction::Get, &workspace_b))
            .unwrap()
            .items
            .is_empty());

        let path = store.plan_file_path(workspace_a.to_str().unwrap()).unwrap();
        fs::write(&path, b"not-json").unwrap();
        assert!(matches!(
            store.execute(request(PlanAction::Get, &workspace_a)),
            Err(PlanError::Corrupt(_))
        ));

        assert!(
            store
                .execute(request(PlanAction::Clear, &workspace_a))
                .unwrap()
                .changed
        );

        let mut recover = request(PlanAction::Set, &workspace_a);
        recover.items = Some(items());
        assert!(store.execute(recover).unwrap().changed);
        assert_eq!(
            store
                .execute(request(PlanAction::Get, &workspace_a))
                .unwrap()
                .items
                .len(),
            2
        );
    }

    #[test]
    fn invalid_items_and_unknown_ids_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = PlanStore::new(temp.path().join("plans"));

        let mut duplicate = request(PlanAction::Set, &workspace);
        let mut duplicate_items = items();
        duplicate_items[1].id = "step-1".to_string();
        duplicate.items = Some(duplicate_items);
        assert!(matches!(
            store.execute(duplicate),
            Err(PlanError::Validation(_))
        ));

        let mut empty_text = request(PlanAction::Set, &workspace);
        let mut empty_text_items = items();
        empty_text_items[0].text = "  ".to_string();
        empty_text.items = Some(empty_text_items);
        assert!(matches!(
            store.execute(empty_text),
            Err(PlanError::Validation(_))
        ));

        let mut multiple_active = request(PlanAction::Set, &workspace);
        let mut multiple_active_items = items();
        multiple_active_items[0].status = PlanStatus::InProgress;
        multiple_active_items[1].status = PlanStatus::InProgress;
        multiple_active.items = Some(multiple_active_items);
        assert!(matches!(
            store.execute(multiple_active),
            Err(PlanError::Validation(_))
        ));

        let mut set = request(PlanAction::Set, &workspace);
        set.items = Some(items());
        store.execute(set).unwrap();

        let mut unknown = request(PlanAction::Update, &workspace);
        unknown.id = Some("missing".to_string());
        unknown.status = Some(PlanStatus::InProgress);
        assert!(matches!(
            store.execute(unknown),
            Err(PlanError::Conflict(_))
        ));
    }

    #[test]
    fn concurrent_duplicate_update_is_serialized() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = Arc::new(PlanStore::new(temp.path().join("plans")));

        let mut set = request(PlanAction::Set, &workspace);
        set.items = Some(items());
        store.execute(set).unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let workspace = workspace.clone();
                thread::spawn(move || {
                    let mut update = request(PlanAction::Update, &workspace);
                    update.id = Some("step-1".to_string());
                    update.status = Some(PlanStatus::InProgress);
                    barrier.wait();
                    store.execute(update).unwrap().changed
                })
            })
            .collect();
        barrier.wait();
        let changed_count = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|changed| *changed)
            .count();
        assert_eq!(changed_count, 1);
    }
}
