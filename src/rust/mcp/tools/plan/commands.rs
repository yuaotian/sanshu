use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use super::store::{get_plan_store, PlanStore};
use super::types::PlanResult;

struct ActivePlanWatcher {
    _watcher: RecommendedWatcher,
    workspace: String,
}

#[derive(Default)]
pub struct PlanWatchState {
    active: Mutex<Option<ActivePlanWatcher>>,
}

#[derive(Clone, Serialize)]
struct PlanUpdatedEvent {
    workspace: String,
}

#[tauri::command]
pub fn get_plan_snapshot(workspace: String) -> Result<PlanResult, String> {
    get_plan_store()
        .and_then(|store| store.get_snapshot(&workspace))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn start_plan_watch(
    workspace: String,
    app: AppHandle,
    state: State<'_, PlanWatchState>,
) -> Result<(), String> {
    let store = get_plan_store().map_err(|error| error.to_string())?;
    store.ensure_root_dir().map_err(|error| error.to_string())?;
    let normalized =
        PlanStore::normalize_workspace(&workspace).map_err(|error| error.to_string())?;
    let target_path = store
        .plan_file_path(&workspace)
        .map_err(|error| error.to_string())?;
    let watch_dir = store.root_dir().to_path_buf();

    let mut active = state
        .active
        .lock()
        .map_err(|_| "计划监听状态锁已损坏".to_string())?;
    if active
        .as_ref()
        .is_some_and(|watcher| watcher.workspace == normalized)
    {
        return Ok(());
    }

    let event_workspace = normalized.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            log::warn!("计划文件监听失败：{:?}", result.err());
            return;
        };

        if event_matches_target(&event.paths, &target_path) {
            if let Err(error) = app.emit(
                "plan-updated",
                PlanUpdatedEvent {
                    workspace: event_workspace.clone(),
                },
            ) {
                log::warn!("发送计划更新事件失败：{}", error);
            }
        }
    })
    .map_err(|error| format!("创建计划文件监听器失败：{}", error))?;
    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|error| format!("监听计划目录 {} 失败：{}", watch_dir.display(), error))?;

    *active = Some(ActivePlanWatcher {
        _watcher: watcher,
        workspace: normalized,
    });
    Ok(())
}

#[tauri::command]
pub fn stop_plan_watch(state: State<'_, PlanWatchState>) -> Result<(), String> {
    let mut active = state
        .active
        .lock()
        .map_err(|_| "计划监听状态锁已损坏".to_string())?;
    active.take();
    Ok(())
}

fn event_matches_target(paths: &[PathBuf], target: &PathBuf) -> bool {
    let target = comparable_path(target);
    paths.iter().any(|path| comparable_path(path) == target)
}

fn comparable_path(path: &PathBuf) -> String {
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
        normalized = format!("//{}", rest);
    } else if let Some(rest) = normalized.strip_prefix("//?/") {
        normalized = rest.to_string();
    }
    #[cfg(windows)]
    {
        normalized = normalized.to_lowercase();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_only_accepts_target_file() {
        let target = PathBuf::from("plans/a.json");
        assert!(event_matches_target(&[target.clone()], &target));
        assert!(!event_matches_target(
            &[PathBuf::from("plans/b.json")],
            &target
        ));

        #[cfg(windows)]
        assert!(event_matches_target(
            &[PathBuf::from(r"\\?\C:\plans\a.json")],
            &PathBuf::from(r"C:\plans\a.json")
        ));
    }
}
