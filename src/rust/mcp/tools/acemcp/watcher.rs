// 文件监听管理：
//
// 设计要点（修复"上次同步 16 天前"卡住问题）：
// 1. 直接使用 `notify::recommended_watcher`（不再用 notify_debouncer_full）。
// 2. 在监听回调里做路径预过滤，过滤掉编译产物、缓存目录、日志等高频写入路径，
//    避免无关事件不断重置防抖计时器，导致回调永远不触发。
// 3. 自管 debounce：用 mpsc 把"通过过滤"的事件推到后台任务，后台任务用
//    `tokio::time::sleep` + `last_event_at` 维护"静默期"门限；同时通过
//    `first_event_at` 维护"最大等待时间"门限，防止用户持续小写入永远触发不了。
// 4. 嵌套项目场景：当变更不属于任何子项目但仍在父项目根之内（且未被排除），
//    把父项目本身加入待索引列表——之前直接吞掉导致父项目长期不更新。

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::types::AcemcpConfig;
use super::mcp::{should_skip_auto_index_for_auth_failure, update_index};
use crate::log_important;
use crate::log_debug;

/// 默认静默期（毫秒）：从 180s → 30s，更贴近真实开发节奏
pub const DEFAULT_DEBOUNCE_MS: u64 = 30_000;
/// 默认最大等待时间（毫秒）：5 分钟兜底，防止持续小写入永远触发不了 flush
pub const DEFAULT_MAX_WAIT_MS: u64 = 300_000;

/// 始终忽略的路径段（按目录名匹配，跨平台通用）
/// 说明：这些目录通常包含构建产物、缓存、依赖或 IDE 元数据，
/// 写入频率极高，且对代码索引无价值。
const ALWAYS_IGNORE_SEGMENTS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    "out",
    "coverage",
    ".next",
    ".nuxt",
    ".vite",
    ".cache",
    ".turbo",
    ".idea",
    ".vscode",
    ".gradle",
    ".mvn",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "__pycache__",
    "venv",
    ".venv",
    "env",
    "logs",
    "log",
    "tmp",
    ".tmp",
];

/// 始终忽略的文件名后缀（按文件名匹配）
const ALWAYS_IGNORE_FILE_SUFFIXES: &[&str] = &[
    ".log",
    ".tmp",
    ".swp",
    ".swo",
    ".lock",
    ".pyc",
    ".class",
    ".DS_Store",
];

/// 规范化项目路径（去除 Windows 扩展路径前缀并统一使用正斜杠）
///
/// 说明：notify / canonicalize 在 Windows 下可能返回 `//?/C:/...` 或 `\\?\\C:\\...`，
/// 这会导致字符串前缀匹配失败（进而无法路由到正确的嵌套子项目）。
fn normalize_project_path(path: &str) -> String {
    let mut p = path.to_string();

    // 处理 //?/ 格式（canonicalize 在某些情况下返回）
    if p.starts_with("//?/") {
        p = p[4..].to_string();
    }
    // 处理 \\?\ 格式（Windows 扩展路径语法）
    else if p.starts_with("\\\\?\\") {
        p = p[4..].to_string();
    }

    // 统一使用正斜杠
    p.replace('\\', "/")
}

/// 嵌套项目监听信息
#[derive(Debug, Clone)]
struct NestedWatchInfo {
    /// 子项目绝对路径（规范化后，使用正斜杠）
    absolute_path: String,
    /// 子项目相对路径
    #[allow(dead_code)]
    relative_path: String,
}

/// 路径过滤器：判断变更路径是否应当忽略
///
/// 内置高频目录黑名单 + 用户配置的 `exclude_patterns`（globset），
/// 命中任意一条都视为"无关变更"，不触发自动索引。
#[derive(Clone)]
pub struct PathFilter {
    /// 监听根目录（规范化、正斜杠）
    root: Arc<String>,
    /// 用户配置 exclude_patterns 对应的 GlobSet（可选）
    user_globset: Option<Arc<GlobSet>>,
}

impl PathFilter {
    pub fn new(root: &str, exclude_patterns: &[String]) -> Self {
        let user_globset = if exclude_patterns.is_empty() {
            None
        } else {
            build_exclude_globset(exclude_patterns).map(Arc::new).ok()
        };
        Self {
            root: Arc::new(root.to_string()),
            user_globset,
        }
    }

    /// 判断给定路径是否应当忽略
    /// - path 可以是 `notify` 上报的绝对路径（可能含扩展前缀/反斜杠）
    /// - 返回 true 表示忽略
    pub fn should_ignore(&self, path: &Path) -> bool {
        let normalized = normalize_project_path(&path.to_string_lossy());

        // 计算相对于监听根的相对路径（无法计算时按文件名兜底）
        let rel = if let Some(stripped) = normalized.strip_prefix(self.root.as_str()) {
            stripped.trim_start_matches('/')
        } else {
            // 不在根之下：通常是临时文件或符号链接，统一忽略
            return true;
        };

        if rel.is_empty() {
            // 根目录自身的事件（如 mtime 更新），忽略以避免无意义触发
            return true;
        }

        // 1) 路径段黑名单
        for seg in rel.split('/') {
            if seg.is_empty() {
                continue;
            }
            if ALWAYS_IGNORE_SEGMENTS.iter().any(|i| i.eq_ignore_ascii_case(seg)) {
                return true;
            }
        }

        // 2) 文件名后缀黑名单
        let last = rel.rsplit('/').next().unwrap_or("");
        for suffix in ALWAYS_IGNORE_FILE_SUFFIXES {
            if last.ends_with(suffix) {
                return true;
            }
        }

        // 3) 用户 exclude_patterns（与索引层一致，使用 globset）
        if let Some(gs) = &self.user_globset {
            if gs.is_match(rel) {
                return true;
            }
            // 兼容"按段匹配"语义（与 mcp.rs 的 should_exclude 保持一致）
            for seg in rel.split('/') {
                if seg.is_empty() {
                    continue;
                }
                if gs.is_match(seg) {
                    return true;
                }
            }
        }

        false
    }
}

/// 构建 exclude 模式的 GlobSet（与 mcp.rs::build_exclude_globset 等价的本地版本，避免循环依赖）
fn build_exclude_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            builder.add(g);
        }
    }
    builder.build().map_err(|e| anyhow::anyhow!("构建 exclude globset 失败: {}", e))
}

/// 单个项目的监听句柄：包含 `notify` 监听器 + 后台 debounce 任务的取消信号
struct WatchHandle {
    _watcher: RecommendedWatcher,
    /// 用于通知后台 debounce 任务停止
    cancel_tx: tokio::sync::oneshot::Sender<()>,
}

impl WatchHandle {
    fn shutdown(self) {
        // _watcher 在 drop 时会停止底层监听
        let _ = self.cancel_tx.send(());
    }
}

/// 文件监听器管理器
/// 负责管理多个项目的文件监听器
pub struct WatcherManager {
    /// 项目路径 -> 监听句柄
    watchers: Arc<Mutex<HashMap<String, WatchHandle>>>,
    /// 是否启用自动索引（全局开关）
    auto_index_enabled: Arc<Mutex<bool>>,
    /// 父目录 -> 嵌套子项目列表（用于智能路由文件变更到正确的子项目）
    nested_project_map: Arc<Mutex<HashMap<String, Vec<NestedWatchInfo>>>>,
}

impl WatcherManager {
    /// 创建新的监听器管理器
    pub fn new() -> Self {
        // 从配置读取全局自动索引开关（默认启用）
        // 说明：该开关需要跨重启生效，因此不再仅依赖进程内状态
        let enabled_from_config = crate::config::load_standalone_config()
            .ok()
            .and_then(|c| c.mcp_config.acemcp_auto_index_enabled)
            .unwrap_or(true);
        log_debug!("初始化自动索引开关: {}", enabled_from_config);

        Self {
            watchers: Arc::new(Mutex::new(HashMap::new())),
            auto_index_enabled: Arc::new(Mutex::new(enabled_from_config)),
            nested_project_map: Arc::new(Mutex::new(HashMap::new())),
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

    /// 检测项目下的嵌套 Git 子项目
    fn detect_nested_projects(&self, project_root: &str) -> Vec<NestedWatchInfo> {
        // 调用 mcp.rs 中的嵌套项目检测逻辑
        match super::mcp::AcemcpTool::get_project_with_nested_status(project_root.to_string()) {
            Ok(status) => {
                status.nested_projects
                    .into_iter()
                    .map(|np| NestedWatchInfo {
                        absolute_path: np.absolute_path,
                        relative_path: np.relative_path,
                    })
                    .collect()
            }
            Err(e) => {
                log_debug!("检测嵌套项目失败: {}", e);
                Vec::new()
            }
        }
    }

    /// 根据变更文件路径确定需要索引的项目（最长前缀匹配 + 父项目兜底）
    /// - 当变更不属于任何子项目时，回退到父项目本身（修复嵌套场景丢事件的 bug）
    /// - 路径若被 `PathFilter` 过滤掉，则不参与计算（外层应已过滤）
    fn determine_affected_projects(
        parent_root: &str,
        changed_paths: &[PathBuf],
        nested_infos: &[NestedWatchInfo],
    ) -> Vec<String> {
        if nested_infos.is_empty() {
            return vec![parent_root.to_string()];
        }

        let mut affected: HashSet<String> = HashSet::new();

        for path in changed_paths {
            let path_str = normalize_project_path(&path.to_string_lossy());

            // 找到包含此路径的子项目（最长前缀匹配）
            let mut matched_project: Option<&str> = None;
            let mut matched_len = 0;

            for info in nested_infos {
                let prefix = info.absolute_path.as_str();
                let is_prefix = path_str.starts_with(prefix);
                let boundary_ok = path_str.len() == prefix.len()
                    || path_str.as_bytes().get(prefix.len()) == Some(&b'/');

                if is_prefix && boundary_ok && prefix.len() > matched_len {
                    matched_project = Some(&info.absolute_path);
                    matched_len = prefix.len();
                }
            }

            if let Some(project) = matched_project {
                affected.insert(project.to_string());
            } else if path_str.starts_with(parent_root) {
                // 兜底：变更在父项目内但不属于任何子项目
                // 旧实现这里直接吞掉，导致父项目长期不被索引（last_success_time 不更新）
                affected.insert(parent_root.to_string());
            } else {
                log_debug!("文件变更不在父项目范围内，跳过: {}", path_str);
            }
        }

        affected.into_iter().collect()
    }

    /// 为指定项目启动文件监听（支持嵌套子项目智能路由）
    /// 如果已经在监听，则不重复启动
    /// - debounce_ms: 静默期门限（默认 30s）
    /// - max_wait_ms: 最大等待门限（默认 5min），防止持续小写入导致永远不 flush
    pub async fn start_watching(
        &self,
        project_root: String,
        config: AcemcpConfig,
        debounce_ms: Option<u64>,
        max_wait_ms: Option<u64>,
    ) -> Result<()> {
        // 检查全局开关
        if !self.is_auto_index_enabled() {
            log_debug!("全局自动索引已禁用，跳过启动文件监听");
            return Ok(());
        }

        // 规范化路径（用于 key）+ 使用 canonical 路径作为 watcher 监听路径
        let watch_path = PathBuf::from(&project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&project_root));
        let normalized_root = normalize_project_path(&watch_path.to_string_lossy());

        // 检查是否已经在监听
        {
            let watchers = self.watchers.lock().unwrap();
            if watchers.contains_key(&normalized_root) {
                log_debug!("项目 {} 已在监听中，跳过重复启动", normalized_root);
                return Ok(());
            }
        }

        let quiet_ms = debounce_ms.unwrap_or(DEFAULT_DEBOUNCE_MS);
        let max_wait_ms = max_wait_ms.unwrap_or(DEFAULT_MAX_WAIT_MS);
        log_important!(
            info,
            "启动文件监听: project_root={}, quiet_ms={}, max_wait_ms={}",
            normalized_root,
            quiet_ms,
            max_wait_ms
        );

        // 读取嵌套项目索引开关（默认启用）
        let index_nested = crate::config::load_standalone_config()
            .ok()
            .and_then(|c| c.mcp_config.acemcp_index_nested_projects)
            .unwrap_or(true);

        // 检测嵌套子项目
        let nested_infos = if index_nested {
            let infos = self.detect_nested_projects(&normalized_root);
            if !infos.is_empty() {
                log_important!(
                    info,
                    "检测到 {} 个嵌套 Git 子项目，将启用智能路由: {:?}",
                    infos.len(),
                    infos.iter().map(|i| &i.relative_path).collect::<Vec<_>>()
                );
                let mut map = self.nested_project_map.lock().unwrap();
                map.insert(normalized_root.clone(), infos.clone());
            }
            infos
        } else {
            Vec::new()
        };

        // 构建路径过滤器（在监听回调里使用）
        let exclude_patterns = config.exclude_patterns.clone().unwrap_or_else(|| {
            vec![
                "node_modules".to_string(),
                ".git".to_string(),
                "target".to_string(),
                "dist".to_string(),
            ]
        });
        let filter = PathFilter::new(&normalized_root, &exclude_patterns);

        // 事件通道：closure → debounce task
        let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();

        // 创建 notify 原生监听器（不再用 notify_debouncer_full）
        // closure 内做路径过滤，过滤掉的事件不会进入下游 → 不会重置 debounce 计时器
        let filter_for_handler = filter.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(event) => {
                    if !is_meaningful_event_kind(&event.kind) {
                        return;
                    }
                    for p in &event.paths {
                        if filter_for_handler.should_ignore(p) {
                            continue;
                        }
                        // unbounded：失败说明接收端已关闭，监听器即将销毁
                        let _ = tx.send(p.clone());
                    }
                }
                Err(e) => {
                    log_debug!("文件监听错误: {:?}", e);
                }
            }
        })?;

        // 添加监听路径
        watcher.watch(&watch_path, RecursiveMode::Recursive)?;
        log_important!(info, "文件监听已启动: {}", normalized_root);

        // 启动后台 debounce + 索引任务
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let project_root_clone = normalized_root.clone();
        let config_fallback = config.clone();
        let nested_infos_clone = nested_infos.clone();
        tokio::spawn(debounce_and_index_loop(
            rx,
            cancel_rx,
            project_root_clone,
            config_fallback,
            nested_infos_clone,
            Duration::from_millis(quiet_ms),
            Duration::from_millis(max_wait_ms),
        ));

        // 保存监听句柄
        {
            let mut watchers = self.watchers.lock().unwrap();
            watchers.insert(
                normalized_root.clone(),
                WatchHandle {
                    _watcher: watcher,
                    cancel_tx,
                },
            );
        }

        Ok(())
    }

    /// 停止监听指定项目
    pub fn stop_watching(&self, project_root: &str) -> Result<()> {
        let normalized_root = normalize_project_path(
            &PathBuf::from(project_root)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(project_root))
                .to_string_lossy(),
        );

        {
            let mut map = self.nested_project_map.lock().unwrap();
            map.remove(&normalized_root);
        }

        let removed = {
            let mut watchers = self.watchers.lock().unwrap();
            watchers.remove(&normalized_root)
        };

        if let Some(handle) = removed {
            handle.shutdown();
            log_important!(info, "已停止文件监听: {}", normalized_root);
        } else {
            log_debug!("项目 {} 未在监听中", normalized_root);
        }
        Ok(())
    }

    /// 停止所有监听
    pub fn stop_all(&self) {
        {
            let mut map = self.nested_project_map.lock().unwrap();
            map.clear();
        }

        let drained: Vec<(String, WatchHandle)> = {
            let mut watchers = self.watchers.lock().unwrap();
            watchers.drain().collect()
        };

        let count = drained.len();
        for (_root, handle) in drained {
            handle.shutdown();
        }
        log_important!(info, "已停止所有文件监听，共 {} 个项目", count);
    }

    /// 获取当前正在监听的项目列表
    pub fn get_watching_projects(&self) -> Vec<String> {
        let watchers = self.watchers.lock().unwrap();
        watchers.keys().cloned().collect()
    }

    /// 检查指定项目是否正在监听
    pub fn is_watching(&self, project_root: &str) -> bool {
        let normalized_root = normalize_project_path(
            &PathBuf::from(project_root)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(project_root))
                .to_string_lossy(),
        );

        let watchers = self.watchers.lock().unwrap();
        watchers.contains_key(&normalized_root)
    }
}

/// 判断 notify 事件类型是否值得处理
/// 排除 Access、AnyOther 等噪音事件
fn is_meaningful_event_kind(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// 自管 debounce + 触发索引的后台循环
/// - 静默期 quiet_dur：从最后一次事件起静默达到该时长后 flush
/// - 最大等待 max_wait_dur：自第一次事件起累积超过该时长强制 flush
async fn debounce_and_index_loop(
    mut rx: mpsc::UnboundedReceiver<PathBuf>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
    parent_root: String,
    config_fallback: AcemcpConfig,
    nested_infos: Vec<NestedWatchInfo>,
    quiet_dur: Duration,
    max_wait_dur: Duration,
) {
    let mut buffer: Vec<PathBuf> = Vec::new();
    // 用 HashSet 做去重 key（按规范化路径），避免缓冲区里重复同一文件
    let mut seen: HashSet<String> = HashSet::new();
    let mut first_event_at: Option<Instant> = None;
    let mut last_event_at: Option<Instant> = None;

    loop {
        // 计算下一次轮询要等多久：
        // - 缓冲区为空：长睡（被 channel 唤醒）
        // - 有事件：在 quiet 与 max_wait 之间取较短者
        let now = Instant::now();
        let timeout = match (first_event_at, last_event_at) {
            (Some(first), Some(last)) => {
                let until_quiet = quiet_dur.saturating_sub(now.saturating_duration_since(last));
                let until_max = max_wait_dur.saturating_sub(now.saturating_duration_since(first));
                let pick = until_quiet.min(until_max);
                Some(pick.max(Duration::from_millis(50)))
            }
            _ => None, // 无事件时走 None 分支，靠 channel 唤醒
        };

        tokio::select! {
            biased;

            _ = &mut cancel_rx => {
                log_debug!("debounce loop 收到取消信号: project_root={}", parent_root);
                break;
            }

            maybe_path = rx.recv() => {
                match maybe_path {
                    Some(p) => {
                        let key = normalize_project_path(&p.to_string_lossy());
                        if seen.insert(key) {
                            buffer.push(p);
                        }
                        let now = Instant::now();
                        if first_event_at.is_none() {
                            first_event_at = Some(now);
                        }
                        last_event_at = Some(now);
                    }
                    None => {
                        log_debug!("debounce loop channel 关闭: project_root={}", parent_root);
                        break;
                    }
                }
            }

            _ = sleep_optional(timeout), if timeout.is_some() => {
                let now = Instant::now();
                let quiet_ok = last_event_at
                    .map(|t| now.saturating_duration_since(t) >= quiet_dur)
                    .unwrap_or(false);
                let max_ok = first_event_at
                    .map(|t| now.saturating_duration_since(t) >= max_wait_dur)
                    .unwrap_or(false);

                if (quiet_ok || max_ok) && !buffer.is_empty() {
                    let paths: Vec<PathBuf> = std::mem::take(&mut buffer);
                    seen.clear();
                    first_event_at = None;
                    last_event_at = None;

                    log_important!(
                        info,
                        "触发自动索引更新: parent_root={}, paths={}, reason={}",
                        parent_root,
                        paths.len(),
                        if max_ok { "max_wait" } else { "quiet" }
                    );

                    flush_index(&parent_root, &paths, &nested_infos, &config_fallback).await;
                }
            }
        }
    }
}

/// 可选 sleep：当 timeout 为 None 时永久挂起，由 select 的其他分支唤醒
async fn sleep_optional(timeout: Option<Duration>) {
    match timeout {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending::<()>().await,
    }
}

/// 执行一次"flush"：根据变更路径计算受影响项目，依次调用 update_index
async fn flush_index(
    parent_root: &str,
    changed_paths: &[PathBuf],
    nested_infos: &[NestedWatchInfo],
    config_fallback: &AcemcpConfig,
) {
    let projects_to_index =
        WatcherManager::determine_affected_projects(parent_root, changed_paths, nested_infos);

    if projects_to_index.is_empty() {
        log_debug!("无需索引的项目: parent_root={}", parent_root);
        return;
    }

    log_important!(
        info,
        "智能路由: 将索引 {} 个项目: {:?}",
        projects_to_index.len(),
        projects_to_index
    );

    // 每次触发时读取最新配置，避免使用过期的 token
    let latest_config = match super::mcp::AcemcpTool::get_acemcp_config().await {
        Ok(c) => c,
        Err(e) => {
            log_debug!("获取最新 acemcp 配置失败，将使用启动监听时的配置: {}", e);
            config_fallback.clone()
        }
    };

    for project_path in projects_to_index {
        if should_skip_auto_index_for_auth_failure(&latest_config, &project_path) {
            log_important!(
                info,
                "跳过自动索引：检测到 Token 认证失败，需用户手动更新配置: project={}",
                project_path
            );
            continue;
        }

        match update_index(&latest_config, &project_path).await {
            Ok(blob_names) => {
                log_important!(
                    info,
                    "自动索引更新成功: project={}, blobs={}",
                    project_path,
                    blob_names.len()
                );
            }
            Err(e) => {
                log_important!(
                    info,
                    "自动索引更新失败: project={}, error={}",
                    project_path,
                    e
                );
            }
        }
    }
}

/// 全局监听器管理器实例
static WATCHER_MANAGER: once_cell::sync::Lazy<WatcherManager> =
    once_cell::sync::Lazy::new(|| WatcherManager::new());

/// 获取全局监听器管理器
pub fn get_watcher_manager() -> &'static WatcherManager {
    &WATCHER_MANAGER
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn pf() -> PathFilter {
        PathFilter::new(
            "C:/proj",
            &vec![
                "*.lock".to_string(),
                "secrets/*".to_string(),
            ],
        )
    }

    #[test]
    fn always_ignore_segments_match() {
        let f = pf();
        assert!(f.should_ignore(Path::new("C:\\proj\\target\\debug\\foo.rlib")));
        assert!(f.should_ignore(Path::new("C:/proj/node_modules/x/y.js")));
        assert!(f.should_ignore(Path::new("C:/proj/.git/objects/aa/bb")));
        assert!(f.should_ignore(Path::new("C:/proj/dist/index.js")));
        assert!(f.should_ignore(Path::new("C:/proj/sub/.vite/deps_temp/_metadata.json")));
        assert!(f.should_ignore(Path::new("C:/proj/__pycache__/foo.pyc")));
    }

    #[test]
    fn meaningful_paths_pass_through() {
        let f = pf();
        assert!(!f.should_ignore(Path::new("C:/proj/src/main.rs")));
        assert!(!f.should_ignore(Path::new("C:\\proj\\src\\frontend\\App.vue")));
        assert!(!f.should_ignore(Path::new("C:/proj/README.md")));
    }

    #[test]
    fn user_glob_excludes_take_effect() {
        let f = pf();
        // *.lock 命中
        assert!(f.should_ignore(Path::new("C:/proj/Cargo.lock")));
        // secrets/* 命中（按段匹配）
        assert!(f.should_ignore(Path::new("C:/proj/secrets/api.key")));
    }

    #[test]
    fn out_of_root_is_ignored() {
        let f = pf();
        assert!(f.should_ignore(Path::new("D:/other/foo.rs")));
    }

    #[test]
    fn extension_only_files_are_ignored() {
        let f = pf();
        assert!(f.should_ignore(Path::new("C:/proj/build.log")));
        assert!(f.should_ignore(Path::new("C:/proj/.DS_Store")));
        assert!(f.should_ignore(Path::new("C:/proj/foo.swp")));
    }

    #[test]
    fn determine_affected_projects_no_nested() {
        let parent = "C:/proj";
        let nested: Vec<NestedWatchInfo> = vec![];
        let paths = vec![PathBuf::from("C:/proj/src/main.rs")];
        let res = WatcherManager::determine_affected_projects(parent, &paths, &nested);
        assert_eq!(res, vec![parent.to_string()]);
    }

    #[test]
    fn determine_affected_projects_nested_match() {
        let parent = "C:/proj";
        let nested = vec![
            NestedWatchInfo {
                absolute_path: "C:/proj/sub-a".to_string(),
                relative_path: "sub-a".to_string(),
            },
            NestedWatchInfo {
                absolute_path: "C:/proj/sub-b".to_string(),
                relative_path: "sub-b".to_string(),
            },
        ];
        let paths = vec![
            PathBuf::from("C:/proj/sub-a/src/lib.rs"),
            PathBuf::from("C:/proj/sub-b/index.js"),
        ];
        let res = WatcherManager::determine_affected_projects(parent, &paths, &nested);
        let set: HashSet<_> = res.into_iter().collect();
        assert!(set.contains("C:/proj/sub-a"));
        assert!(set.contains("C:/proj/sub-b"));
    }

    #[test]
    fn determine_affected_projects_falls_back_to_parent() {
        // 修复点：变更不在任何子项目内但在父项目内 → 应回落到父项目
        let parent = "C:/proj";
        let nested = vec![NestedWatchInfo {
            absolute_path: "C:/proj/vendor/lib".to_string(),
            relative_path: "vendor/lib".to_string(),
        }];
        let paths = vec![PathBuf::from("C:/proj/src/main.rs")];
        let res = WatcherManager::determine_affected_projects(parent, &paths, &nested);
        assert_eq!(res, vec![parent.to_string()]);
    }

    #[test]
    fn determine_affected_projects_boundary() {
        // 边界：sub-a-extra 不应被误判为 sub-a
        let parent = "C:/proj";
        let nested = vec![NestedWatchInfo {
            absolute_path: "C:/proj/sub-a".to_string(),
            relative_path: "sub-a".to_string(),
        }];
        let paths = vec![PathBuf::from("C:/proj/sub-a-extra/file.rs")];
        let res = WatcherManager::determine_affected_projects(parent, &paths, &nested);
        // 没有被错误归到 sub-a，回落到父项目
        assert_eq!(res, vec![parent.to_string()]);
    }
}
