use anyhow::Result;
use notify_debouncer_full::{
    new_debouncer,
    notify::{RecommendedWatcher, RecursiveMode, Watcher},
    DebounceEventResult, Debouncer, FileIdMap,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

use super::types::AcemcpConfig;
use super::mcp::update_index;
use crate::log_important;
use crate::log_debug;

/// 文件监听器管理器
/// 负责管理多个项目的文件监听器
pub struct WatcherManager {
    /// 项目路径 -> 监听器句柄
    watchers: Arc<Mutex<HashMap<String, Debouncer<RecommendedWatcher, FileIdMap>>>>,
    /// 是否启用自动索引（全局开关）
    auto_index_enabled: Arc<Mutex<bool>>,
}

impl WatcherManager {
    /// 创建新的监听器管理器
    pub fn new() -> Self {
        Self {
            watchers: Arc::new(Mutex::new(HashMap::new())),
            auto_index_enabled: Arc::new(Mutex::new(true)), // 默认启用
        }
    }

    /// 获取全局自动索引开关状态
    pub fn is_auto_index_enabled(&self) -> bool {
        *self.auto_index_enabled.lock().unwrap()
    }

    /// 设置全局自动索引开关
    pub fn set_auto_index_enabled(&self, enabled: bool) {
        *self.auto_index_enabled.lock().unwrap() = enabled;
        log_important!(info, "全局自动索引开关已{}",  if enabled { "启用" } else { "禁用" });
    }

    /// 为指定项目启动文件监听
    /// 如果已经在监听，则不重复启动
    pub async fn start_watching(&self, project_root: String, config: AcemcpConfig) -> Result<()> {
        // 检查全局开关
        if !self.is_auto_index_enabled() {
            log_debug!("全局自动索引已禁用，跳过启动文件监听");
            return Ok(());
        }

        let normalized_root = PathBuf::from(&project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&project_root))
            .to_string_lossy()
            .replace('\\', "/");

        // 检查是否已经在监听
        {
            let watchers = self.watchers.lock().unwrap();
            if watchers.contains_key(&normalized_root) {
                log_debug!("项目 {} 已在监听中，跳过重复启动", normalized_root);
                return Ok(());
            }
        }

        log_important!(info, "启动文件监听: project_root={}", normalized_root);

        // 创建异步通道用于接收文件变更事件
        let (tx, mut rx) = mpsc::channel::<()>(100);

        // 创建 debouncer（1.5 秒延迟）
        let mut debouncer = new_debouncer(
            Duration::from_millis(1500),
            None,
            move |result: DebounceEventResult| {
                match result {
                    Ok(events) => {
                        if !events.is_empty() {
                            log_debug!("检测到文件变更事件，共 {} 个", events.len());
                            // 发送信号触发索引更新
                            let _ = tx.try_send(());
                        }
                    }
                    Err(errors) => {
                        log_debug!("文件监听错误: {:?}", errors);
                    }
                }
            },
        )?;

        // 添加监听路径
        debouncer
            .watcher()
            .watch(Path::new(&project_root), RecursiveMode::Recursive)?;

        log_important!(info, "文件监听已启动: {}", normalized_root);

        // 保存 debouncer 到管理器
        {
            let mut watchers = self.watchers.lock().unwrap();
            watchers.insert(normalized_root.clone(), debouncer);
        }

        // 启动后台任务处理索引更新
        let project_root_clone = normalized_root.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            while let Some(_) = rx.recv().await {
                log_important!(info, "触发自动索引更新: project_root={}", project_root_clone);
                
                match update_index(&config_clone, &project_root_clone).await {
                    Ok(blob_names) => {
                        log_important!(info, "自动索引更新成功: project_root={}, blobs={}", project_root_clone, blob_names.len());
                    }
                    Err(e) => {
                        log_important!(info, "自动索引更新失败: project_root={}, error={}", project_root_clone, e);
                    }
                }
            }
        });

        Ok(())
    }

    /// 停止监听指定项目
    pub fn stop_watching(&self, project_root: &str) -> Result<()> {
        let normalized_root = PathBuf::from(project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root))
            .to_string_lossy()
            .replace('\\', "/");

        let mut watchers = self.watchers.lock().unwrap();
        if watchers.remove(&normalized_root).is_some() {
            log_important!(info, "已停止文件监听: {}", normalized_root);
            Ok(())
        } else {
            log_debug!("项目 {} 未在监听中", normalized_root);
            Ok(())
        }
    }

    /// 停止所有监听
    pub fn stop_all(&self) {
        let mut watchers = self.watchers.lock().unwrap();
        let count = watchers.len();
        watchers.clear();
        log_important!(info, "已停止所有文件监听，共 {} 个项目", count);
    }

    /// 获取当前正在监听的项目列表
    pub fn get_watching_projects(&self) -> Vec<String> {
        let watchers = self.watchers.lock().unwrap();
        watchers.keys().cloned().collect()
    }

    /// 检查指定项目是否正在监听
    pub fn is_watching(&self, project_root: &str) -> bool {
        let normalized_root = PathBuf::from(project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root))
            .to_string_lossy()
            .replace('\\', "/");

        let watchers = self.watchers.lock().unwrap();
        watchers.contains_key(&normalized_root)
    }
}

/// 全局监听器管理器实例
static WATCHER_MANAGER: once_cell::sync::Lazy<WatcherManager> =
    once_cell::sync::Lazy::new(|| WatcherManager::new());

/// 获取全局监听器管理器
pub fn get_watcher_manager() -> &'static WatcherManager {
    &WATCHER_MANAGER
}

