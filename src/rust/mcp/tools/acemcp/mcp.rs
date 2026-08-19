use anyhow::Result;
use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use encoding_rs::{GBK, UTF_8, WINDOWS_1252};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use ring::digest::{Context as ShaContext, SHA256};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::AppHandle;

use super::jobs::{
    self, IndexJob, JOB_COLLECTING, JOB_COMPLETED, JOB_FAILED, JOB_PAUSED, JOB_QUEUED,
    JOB_SCOPE_BLOCKED, JOB_UPLOADING,
};
use super::scope_guard::{
    critical_path_risk, effective_exclude_patterns, is_confirmed_project_root,
    preflight_project_scope,
};
use super::types::{
    AcemcpConfig, AcemcpRequest, FileIndexStatus, FileIndexStatusKind, IndexStatus,
    NestedProjectInfo, ProjectFilesStatus, ProjectIndexStatus, ProjectScopeRisk,
    ProjectWithNestedStatus, ProjectsIndexStatus,
};
use crate::log_debug;
use crate::log_important;
// 代理模块（在 create_acemcp_client 中使用）

/// Acemcp工具实现
pub struct AcemcpTool;

/// 记录当前进程内已启动的后台索引任务，避免首次并发搜索时重复触发。
static AUTO_INDEX_INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static PROJECTS_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROJECTS_STATUS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn auto_index_inflight() -> &'static Mutex<HashSet<String>> {
    AUTO_INDEX_INFLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

fn projects_file_lock() -> &'static Mutex<()> {
    PROJECTS_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

fn projects_status_lock() -> &'static Mutex<()> {
    PROJECTS_STATUS_LOCK.get_or_init(|| Mutex::new(()))
}

impl AcemcpTool {
    /// 执行代码库搜索
    /// 当检测到索引缺失或失效时，会在后台自动启动索引/重建任务
    pub async fn search_context(request: AcemcpRequest) -> Result<CallToolResult, McpError> {
        log_important!(
            info,
            "Acemcp搜索请求（仅搜索模式）: project_root_path={}, query={}",
            request.project_root_path,
            request.query
        );

        // 读取配置
        let mut acemcp_config = Self::get_acemcp_config()
            .await
            .map_err(|e| McpError::internal_error(format!("获取acemcp配置失败: {}", e), None))?;

        // 规范化 base_url（缺协议时补 http://），并去除末尾斜杠
        if let Some(base) = &acemcp_config.base_url {
            let normalized = normalize_base_url(base);
            acemcp_config.base_url = Some(normalized);
        }

        // 首次搜索时自动启动文件监听（如果尚未启动）
        let watcher_manager = super::watcher::get_watcher_manager();
        if !watcher_manager.is_watching(&request.project_root_path) {
            log_debug!("首次搜索，尝试启动文件监听");
            if let Err(e) = watcher_manager
                .start_watching(
                    request.project_root_path.clone(),
                    acemcp_config.clone(),
                    None, // 静默期：使用默认 30s
                    None, // 最大等待：使用默认 5min
                )
                .await
            {
                if get_project_status(&request.project_root_path)
                    .scope_risk
                    .is_some()
                {
                    return Ok(CallToolResult {
                        content: vec![Content::text(
                            "代码搜索已暂停：项目路径或索引规模存在风险。请打开等一下窗口确认该项目路径，或移除错误的索引记录。",
                        )],
                        is_error: Some(true),
                        meta: None,
                        structured_content: None,
                    });
                }
                log_debug!("启动文件监听失败（不影响搜索）: {}", e);
            }
        }

        // 1. 检查初始索引状态
        let initial_state = get_initial_index_state(&request.project_root_path);
        log_debug!("项目索引状态: {:?}", initial_state);

        // 2. 根据状态执行相应操作
        let mut hint_message = String::new();
        match initial_state {
            InitialIndexState::Missing | InitialIndexState::Idle | InitialIndexState::Failed => {
                // 启动后台索引
                if let Err(e) =
                    ensure_initial_index_background(&acemcp_config, &request.project_root_path)
                        .await
                {
                    log_debug!("启动后台索引失败（不影响搜索）: {}", e);
                }
            }
            InitialIndexState::Indexing => {
                // 正在索引中，应用智能等待
                if let Some((min_wait, max_wait)) = acemcp_config.smart_wait_range {
                    let wait_secs = fastrand::u64(min_wait..=max_wait);

                    log_important!(
                        info,
                        "检测到索引正在进行中，智能等待 {} 秒后执行搜索",
                        wait_secs
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;

                    hint_message = format!(
                        "\n\n💡 提示：检测到索引正在进行中，已等待 {} 秒以获取更完整的搜索结果。",
                        wait_secs
                    );
                }
            }
            InitialIndexState::Synced => {
                // 已完成索引，直接搜索
                log_debug!("项目索引已完成，直接执行搜索");
            }
        }

        // 3. 执行搜索或返回索引中提示
        let search_result =
            match search_only(&acemcp_config, &request.project_root_path, &request.query).await {
                Ok(text) => text,
                Err(e) => {
                    let error_text = e.to_string();
                    let display_text = if error_text.starts_with("代码搜索失败：") {
                        error_text
                    } else {
                        format!("Acemcp搜索失败: {}", error_text)
                    };
                    return Ok(CallToolResult {
                        content: vec![Content::text(display_text)],
                        is_error: Some(true),
                        meta: None,
                        structured_content: None,
                    });
                }
            };

        // 4. 附加提示信息
        let final_result = if hint_message.is_empty() {
            search_result
        } else {
            format!("{}{}", search_result, hint_message)
        };

        Ok(CallToolResult {
            content: vec![Content::text(final_result)],
            is_error: None,
            meta: None,
            structured_content: None,
        })
    }

    /// 执行索引更新（向后兼容的索引+搜索一体化接口）
    pub async fn index_and_search_legacy(
        request: AcemcpRequest,
    ) -> Result<CallToolResult, McpError> {
        log_important!(
            info,
            "Acemcp索引+搜索请求（兼容模式）: project_root_path={}, query={}",
            request.project_root_path,
            request.query
        );

        // 读取配置
        let mut acemcp_config = Self::get_acemcp_config()
            .await
            .map_err(|e| McpError::internal_error(format!("获取acemcp配置失败: {}", e), None))?;

        // 规范化 base_url（缺协议时补 http://），并去除末尾斜杠
        if let Some(base) = &acemcp_config.base_url {
            let normalized = normalize_base_url(base);
            acemcp_config.base_url = Some(normalized);
        }

        if !ensure_project_scope_allowed(&acemcp_config, &request.project_root_path)
            .await
            .map_err(|error| {
                McpError::internal_error(format!("检查项目索引范围失败: {}", error), None)
            })?
        {
            return Ok(CallToolResult {
                content: vec![Content::text(
                    "索引更新已暂停：项目路径或索引规模存在风险，请在等一下窗口中确认。",
                )],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            });
        }

        // 先执行索引更新
        match update_index(&acemcp_config, &request.project_root_path).await {
            Ok(_blob_names) => {
                // 索引成功后执行搜索
                match search_only(&acemcp_config, &request.project_root_path, &request.query).await
                {
                    Ok(text) => Ok(CallToolResult {
                        content: vec![Content::text(text)],
                        is_error: None,
                        meta: None,
                        structured_content: None,
                    }),
                    Err(e) => {
                        let error_text = e.to_string();
                        let display_text = if error_text.starts_with("代码搜索失败：") {
                            error_text
                        } else {
                            format!("搜索失败: {}", error_text)
                        };
                        Ok(CallToolResult {
                            content: vec![Content::text(display_text)],
                            is_error: Some(true),
                            meta: None,
                            structured_content: None,
                        })
                    }
                }
            }
            Err(e) => Ok(CallToolResult {
                content: vec![Content::text(format!("索引更新失败: {}", e))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            }),
        }
    }

    /// 手动触发索引更新（供 Tauri 命令调用）。这里只提交任务，不等待网络上传。
    pub async fn trigger_index_update(project_root_path: String) -> Result<String> {
        Self::schedule_index_update(project_root_path, IndexJobMode::Incremental, None).await
    }

    /// 提交后台索引任务，并可把任务事件广播到 GUI。
    pub async fn trigger_index_update_with_app(
        project_root_path: String,
        full_rebuild: bool,
        app: Option<AppHandle>,
    ) -> Result<String> {
        Self::schedule_index_update(
            project_root_path,
            if full_rebuild {
                IndexJobMode::Full
            } else {
                IndexJobMode::Incremental
            },
            app,
        )
        .await
    }

    /// 支持级联索引嵌套的 Git 子项目；该函数只负责排队，实际上传在后台执行。
    async fn schedule_index_update(
        project_root_path: String,
        mode: IndexJobMode,
        app: Option<AppHandle>,
    ) -> Result<String> {
        log_important!(
            info,
            "提交后台索引任务: project_root_path={}, mode={}",
            project_root_path,
            mode.as_str()
        );

        let acemcp_config = Self::get_acemcp_config().await?;

        // 读取嵌套项目索引开关（默认启用）
        let index_nested = crate::config::load_standalone_config()
            .ok()
            .and_then(|c| c.mcp_config.acemcp_index_nested_projects)
            .unwrap_or(true);

        // 检测嵌套子项目
        let nested_status = match Self::get_project_with_nested_status(project_root_path.clone()) {
            Ok(status) => status,
            Err(e) => {
                log_debug!("获取嵌套项目状态失败，将直接索引父目录: {}", e);
                let launch = start_background_index_with_mode(
                    &acemcp_config,
                    &project_root_path,
                    true,
                    mode,
                    app,
                )
                .await?;
                return Ok(format!("已提交后台索引任务: {:?}", launch));
            }
        };

        let has_nested = !nested_status.nested_projects.is_empty();

        if has_nested && index_nested {
            // 策略A: 有嵌套子项目且开关启用，只索引子项目，不索引父目录（避免无意义上传）
            log_important!(
                info,
                "检测到 {} 个嵌套 Git 子项目，将分别索引",
                nested_status.nested_projects.len()
            );

            let mut launched = Vec::new();
            for nested in &nested_status.nested_projects {
                let state = start_background_index_with_mode(
                    &acemcp_config,
                    &nested.absolute_path,
                    true,
                    mode,
                    app.clone(),
                )
                .await?;
                launched.push((nested.relative_path.clone(), format!("{:?}", state)));
            }
            Ok(format!("已提交 {} 个子项目后台索引任务: {:?}", launched.len(), launched))
        } else {
            // 策略B: 无嵌套子项目或开关关闭，直接提交父项目后台任务。
            let state = start_background_index_with_mode(
                &acemcp_config,
                &project_root_path,
                true,
                mode,
                app,
            )
            .await?;
            Ok(format!("已提交后台索引任务: {:?}", state))
        }
    }

    /// 获取项目索引状态（供 Tauri 命令调用）
    pub fn get_index_status(project_root_path: String) -> ProjectIndexStatus {
        get_project_status(&project_root_path)
    }

    /// 获取所有项目的索引状态（供 Tauri 命令调用）
    pub fn get_all_index_status() -> ProjectsIndexStatus {
        let mut all_status = load_projects_status();
        // 三份持久化状态互相校验：状态文件丢失时，仍可从已确认 blob 或任务清单补回项目。
        for project_root in load_projects_file().0.into_keys() {
            all_status
                .projects
                .entry(project_root.clone())
                .or_insert_with(|| {
                    let mut status = ProjectIndexStatus::default();
                    status.project_root = project_root;
                    status
                });
        }
        for job in jobs::load_manifest().jobs.into_values() {
            all_status
                .projects
                .entry(job.project_root.clone())
                .or_insert_with(|| {
                    let mut status = ProjectIndexStatus::default();
                    status.project_root = job.project_root;
                    status
                });
        }
        for status in all_status.projects.values_mut() {
            reconcile_project_status_with_job(status);
            enrich_project_scope_state(status);
        }
        all_status
    }

    /// 获取项目内所有可索引文件的索引状态（供 Tauri 命令调用）
    pub async fn get_project_files_status(
        project_root_path: String,
    ) -> anyhow::Result<ProjectFilesStatus> {
        // 读取 Acemcp 配置，主要用于获取扩展名、排除规则和分块行数
        let acemcp_config = Self::get_acemcp_config().await?;
        let max_lines = acemcp_config.max_lines_per_blob.unwrap_or(800) as usize;
        let text_exts = acemcp_config.text_extensions.clone().unwrap_or_default();
        let exclude_patterns =
            effective_exclude_patterns(acemcp_config.exclude_patterns.as_deref());

        // 读取 projects.json，获取已索引的 blob 名称集合
        let projects = load_projects_file();

        // 使用 normalize_project_path 去除 Windows 扩展路径前缀
        let normalized_root = normalize_project_path(
            &PathBuf::from(&project_root_path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&project_root_path))
                .to_string_lossy(),
        );

        let mut existing_blob_names: std::collections::HashSet<String> = projects
            .0
            .get(&normalized_root)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();

        let project_status = get_project_status(&project_root_path);
        let current_scope_hash = build_index_scope_hash(&acemcp_config);
        if is_index_scope_stale(
            &project_status,
            current_scope_hash.as_deref(),
            !existing_blob_names.is_empty(),
        ) {
            // 中文注释：配置变更后，旧 blob 属于其他索引空间，这里按“未索引”展示更符合真实状态。
            existing_blob_names.clear();
        }

        let files = collect_file_statuses(
            &project_root_path,
            &text_exts,
            &exclude_patterns,
            max_lines,
            &existing_blob_names,
        )?;

        Ok(ProjectFilesStatus {
            project_root: normalized_root,
            files,
        })
    }

    /// 获取acemcp配置（公有方法，供 commands 模块调用）
    pub async fn get_acemcp_config() -> Result<AcemcpConfig> {
        // 从配置文件中读取acemcp配置
        let config = crate::config::load_standalone_config()
            .map_err(|e| anyhow::anyhow!("读取配置文件失败: {}", e))?;

        Ok(AcemcpConfig {
            base_url: config.mcp_config.acemcp_base_url,
            token: config.mcp_config.acemcp_token,
            batch_size: config.mcp_config.acemcp_batch_size,
            max_lines_per_blob: config.mcp_config.acemcp_max_lines_per_blob,
            text_extensions: config.mcp_config.acemcp_text_extensions,
            exclude_patterns: config.mcp_config.acemcp_exclude_patterns,
            // 智能等待默认值：1-5 秒随机等待
            smart_wait_range: Some((1, 5)),
            // 代理配置
            proxy_enabled: config.mcp_config.acemcp_proxy_enabled,
            proxy_host: config.mcp_config.acemcp_proxy_host,
            proxy_port: config.mcp_config.acemcp_proxy_port,
            proxy_type: config.mcp_config.acemcp_proxy_type,
            proxy_username: config.mcp_config.acemcp_proxy_username,
            proxy_password: config.mcp_config.acemcp_proxy_password,
        })
    }

    /// 获取工具定义
    pub fn get_tool_definition() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project_root_path": {
                    "type": "string",
                    "description": "项目根目录的绝对路径，使用正斜杠(/)作为分隔符。例如：C:/Users/username/projects/myproject"
                },
                "query": {
                    "type": "string",
                    "description": "用于查找相关代码上下文的自然语言搜索查询。此工具执行语义搜索并返回与查询匹配的代码片段。例如：'日志配置设置初始化logger'（查找日志设置代码）、'用户认证登录'（查找认证相关代码）、'数据库连接池'（查找数据库连接代码）、'错误处理异常'（查找错误处理模式）、'API端点路由'（查找API路由定义）。工具返回带有文件路径和行号的格式化文本片段，显示相关代码的位置。"
                }
            },
            "required": ["project_root_path", "query"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            Tool {
                name: Cow::Borrowed("sou"),
                description: Some(Cow::Borrowed("基于查询在特定项目中搜索相关的代码上下文。依赖后台增量索引与文件监听机制维护索引，并在索引进行中通过智能等待在实时性和响应速度之间做平衡。返回代码库中与查询语义相关的格式化文本片段。")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            }
        } else {
            panic!("Schema creation failed");
        }
    }

    /// 获取项目及其嵌套子项目的索引状态（供 Tauri 命令调用）
    ///
    /// 该方法会扫描项目根目录下的直接子目录，检测哪些是独立的 Git 仓库，
    /// 并返回每个子项目的索引状态。用于前端展示多项目结构。
    pub fn get_project_with_nested_status(
        project_root_path: String,
    ) -> Result<ProjectWithNestedStatus> {
        let root_path = PathBuf::from(&project_root_path);
        // 关键校验：路径不存在时直接返回错误，避免前端静默失败
        if !root_path.exists() || !root_path.is_dir() {
            anyhow::bail!("项目根目录不存在: {}", project_root_path);
        }
        let root_status = get_project_status(&project_root_path);

        let mut nested_projects = Vec::new();
        let mut regular_directories = Vec::new();

        // 从配置读取排除模式，用于过滤嵌套目录（与索引阶段保持一致）
        let configured_excludes = crate::config::load_standalone_config()
            .ok()
            .and_then(|c| c.mcp_config.acemcp_exclude_patterns)
            .unwrap_or_default();
        let exclude_patterns = effective_exclude_patterns(Some(&configured_excludes));
        let exclude_globset = if exclude_patterns.is_empty() {
            None
        } else {
            match build_exclude_globset(&exclude_patterns) {
                Ok(gs) => Some(gs),
                Err(e) => {
                    log_debug!("构建排除模式失败，将忽略目录过滤: {}", e);
                    None
                }
            }
        };

        // 扫描直接子目录（仅第一层）
        let entries =
            fs::read_dir(&root_path).map_err(|e| anyhow::anyhow!("读取项目根目录失败: {}", e))?;
        for entry in entries {
            let entry = entry.map_err(|e| anyhow::anyhow!("读取目录条目失败: {}", e))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // 跳过隐藏目录
            if dir_name.starts_with('.') {
                continue;
            }
            // 使用配置排除目录/路径（支持 glob）
            if should_exclude(&path, &root_path, exclude_globset.as_ref()) {
                continue;
            }

            // 检测是否是 Git 仓库
            let git_dir = path.join(".git");
            let is_git_repo = git_dir.exists() && git_dir.is_dir();

            if is_git_repo {
                // 获取子项目的索引状态
                let sub_path_str = normalize_project_path(&path.to_string_lossy());
                let sub_status = get_project_status(&sub_path_str);

                // 粗略估计文件数量（使用索引状态中的 total_files，如果没有则设为 0）
                let file_count = if sub_status.status != IndexStatus::Idle {
                    sub_status.total_files
                } else {
                    0
                };

                nested_projects.push(NestedProjectInfo {
                    relative_path: dir_name,
                    absolute_path: sub_path_str,
                    is_git_repo: true,
                    index_status: Some(sub_status),
                    file_count,
                });
            } else {
                regular_directories.push(dir_name);
            }
        }

        // 按字母顺序排序
        nested_projects.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        regular_directories.sort();

        Ok(ProjectWithNestedStatus {
            root_status,
            nested_projects,
            regular_directories,
        })
    }
}

// ---------------- 已移除 Python Web 服务依赖，完全使用 Rust 实现 ----------------

// ---------------- 索引初始化状态枚举 ----------------

/// 索引初始化状态
#[derive(Debug, Clone, PartialEq)]
pub enum InitialIndexState {
    /// 项目记录不存在
    Missing,
    /// 从未索引过（状态为 Idle 且 total_files == 0）
    Idle,
    /// 已完成索引
    Synced,
    /// 正在索引中
    Indexing,
    /// 上次索引失败
    Failed,
}

/// 获取项目的初始索引状态
pub fn get_initial_index_state(project_root: &str) -> InitialIndexState {
    let status = get_project_status(project_root);

    match status.status {
        IndexStatus::Idle if status.total_files == 0 => InitialIndexState::Idle,
        IndexStatus::Idle => InitialIndexState::Missing,
        IndexStatus::Synced => InitialIndexState::Synced,
        IndexStatus::Indexing => InitialIndexState::Indexing,
        // 可恢复暂停与普通失败都允许再次触发；具体是否续传由 index_jobs.json 决定。
        IndexStatus::Paused => InitialIndexState::Failed,
        IndexStatus::Failed => InitialIndexState::Failed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundIndexLaunchState {
    Started,
    AlreadyRunning,
    Skipped,
    ScopeBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexJobMode {
    Incremental,
    Full,
}

impl IndexJobMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::Full => "full",
        }
    }

    fn from_str(value: &str) -> Self {
        if value == "full" {
            Self::Full
        } else {
            Self::Incremental
        }
    }
}

fn normalized_config_values(values: Option<&Vec<String>>) -> Vec<String> {
    let mut normalized = values
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

/// ACE 索引空间签名同时覆盖连接身份与会改变 blob 集合的索引参数。
/// 这样扩展名、排除规则或分块大小变化时，也会自动进入全量重建。
pub(crate) fn build_index_scope_hash(config: &AcemcpConfig) -> Option<String> {
    let normalized_base_url = normalize_base_url(config.base_url.as_deref()?);
    let token = config.token.as_deref()?.trim();
    let effective_excludes = effective_exclude_patterns(config.exclude_patterns.as_deref());
    let fingerprint = serde_json::json!({
        "base_url": normalized_base_url,
        "token": token,
        "batch_size": config.batch_size.unwrap_or(10),
        "max_lines_per_blob": config.max_lines_per_blob.unwrap_or(800),
        "text_extensions": normalized_config_values(config.text_extensions.as_ref()),
        "exclude_patterns": normalized_config_values(Some(&effective_excludes)),
    });
    let mut ctx = ShaContext::new(&SHA256);
    ctx.update(fingerprint.to_string().as_bytes());
    Some(hex::encode(ctx.finish().as_ref()))
}

fn require_index_scope_hash(config: &AcemcpConfig) -> anyhow::Result<String> {
    build_index_scope_hash(config).ok_or_else(|| anyhow::anyhow!("未配置完整的 ACE 索引空间身份"))
}

fn is_index_scope_stale(
    status: &ProjectIndexStatus,
    current_scope_hash: Option<&str>,
    has_local_blobs: bool,
) -> bool {
    match current_scope_hash {
        Some(current_hash) => match status.index_scope_hash.as_deref() {
            Some(saved_hash) => saved_hash != current_hash,
            None => has_local_blobs,
        },
        None => false,
    }
}

fn enrich_project_scope_state(status: &mut ProjectIndexStatus) {
    let current_scope_hash = crate::config::load_standalone_config()
        .ok()
        .and_then(|config| {
            build_index_scope_hash(&AcemcpConfig {
                base_url: config.mcp_config.acemcp_base_url,
                token: config.mcp_config.acemcp_token,
                batch_size: config.mcp_config.acemcp_batch_size,
                max_lines_per_blob: config.mcp_config.acemcp_max_lines_per_blob,
                text_extensions: config.mcp_config.acemcp_text_extensions,
                exclude_patterns: config.mcp_config.acemcp_exclude_patterns,
                smart_wait_range: None,
                proxy_enabled: config.mcp_config.acemcp_proxy_enabled,
                proxy_host: config.mcp_config.acemcp_proxy_host,
                proxy_port: config.mcp_config.acemcp_proxy_port,
                proxy_type: config.mcp_config.acemcp_proxy_type,
                proxy_username: config.mcp_config.acemcp_proxy_username,
                proxy_password: config.mcp_config.acemcp_proxy_password,
            })
        });
    let has_local_blobs = if status.project_root.is_empty() {
        false
    } else {
        has_local_blob_names(&status.project_root)
    };
    let stale = is_index_scope_stale(status, current_scope_hash.as_deref(), has_local_blobs);

    status.is_stale = stale;
    status.stale_reason = if stale {
        Some("检测到 ACE 配置已变更，旧索引已失效，等待重新索引".to_string())
    } else {
        None
    };
}

fn record_project_scope_risk(project_root: &str, risk: ProjectScopeRisk) -> anyhow::Result<()> {
    let normalized_root = super::scope_guard::normalize_root(project_root);
    let error_message = format!("索引范围需要确认：{}", risk.reason);
    let _ = jobs::update_job(
        &normalized_root,
        "scope_blocked",
        Some(error_message.clone()),
        |job| {
            job.status = JOB_SCOPE_BLOCKED.to_string();
            job.last_error = Some(error_message.clone());
        },
    );
    update_project_status(&normalized_root, |status| {
        status.status = IndexStatus::Paused;
        status.last_error = Some(error_message.clone());
        status.last_failure_time = Some(chrono::Utc::now());
        status.scope_risk = Some(risk.clone());
    })?;
    log_important!(
        warn,
        "ACE 索引已因项目范围风险暂停: project_root={}, reason_code={}, reason={}",
        normalized_root,
        risk.reason_code,
        risk.reason
    );
    Ok(())
}

pub(crate) fn clear_project_scope_risk(project_root: &str) -> anyhow::Result<()> {
    let normalized_root = super::scope_guard::normalize_root(project_root);
    let blocked_job = jobs::get_job(&normalized_root)
        .is_some_and(|job| job.status == JOB_SCOPE_BLOCKED);
    if get_project_status(&normalized_root).scope_risk.is_none() && !blocked_job {
        return Ok(());
    }
    if blocked_job {
        jobs::remove_job(&normalized_root)?;
    }
    update_project_status(&normalized_root, |status| {
        status.scope_risk = None;
        if status
            .last_error
            .as_deref()
            .is_some_and(|message| message.starts_with("索引范围需要确认："))
        {
            status.last_error = None;
        }
        if status.status == IndexStatus::Paused {
            status.status = IndexStatus::Idle;
        }
    })
}

/// 在启动 watcher 或索引 worker 前完成有界范围预检；已确认路径直接放行。
pub(crate) async fn ensure_project_scope_allowed(
    config: &AcemcpConfig,
    project_root: &str,
) -> anyhow::Result<bool> {
    if is_confirmed_project_root(project_root) {
        clear_project_scope_risk(project_root)?;
        return Ok(true);
    }
    if let Some(risk) = critical_path_risk(project_root) {
        record_project_scope_risk(project_root, risk)?;
        return Ok(false);
    }

    let project_root = project_root.to_string();
    let project_root_for_preflight = project_root.clone();
    let text_extensions = config.text_extensions.clone().unwrap_or_default();
    let exclude_patterns = effective_exclude_patterns(config.exclude_patterns.as_deref());
    let risk = tokio::task::spawn_blocking(move || {
        preflight_project_scope(
            &project_root_for_preflight,
            &text_extensions,
            &exclude_patterns,
        )
    })
    .await
    .map_err(|error| anyhow::anyhow!("项目范围预检任务失败: {}", error))??;

    if let Some(risk) = risk {
        record_project_scope_risk(project_root.as_str(), risk)?;
        return Ok(false);
    }
    clear_project_scope_risk(project_root.as_str())?;
    Ok(true)
}

/// 运行中的任务只保留一个后继请求；全量优先于增量，避免文件事件风暴制造重复任务。
fn request_followup_index(project_root: &str, mode: IndexJobMode) {
    let requested_mode = mode.as_str().to_string();
    let _ = jobs::update_job(
        project_root,
        "rerun_requested",
        Some(format!("已合并后继 {} 索引请求", requested_mode)),
        |job| {
            if job.rerun_mode.as_deref() != Some(IndexJobMode::Full.as_str()) {
                job.rerun_mode = Some(requested_mode.clone());
            }
        },
    );
}

/// 在真正调度异步任务前先做进程内去重，并把项目状态切到 indexing，缩小重复触发窗口。
async fn start_background_index(
    config: &AcemcpConfig,
    project_root: &str,
    force: bool,
) -> anyhow::Result<BackgroundIndexLaunchState> {
    start_background_index_with_mode(
        config,
        project_root,
        force,
        IndexJobMode::Incremental,
        None,
    )
    .await
}

async fn start_background_index_with_mode(
    config: &AcemcpConfig,
    project_root: &str,
    force: bool,
    mode: IndexJobMode,
    app: Option<AppHandle>,
) -> anyhow::Result<BackgroundIndexLaunchState> {
    if !ensure_project_scope_allowed(config, project_root).await? {
        return Ok(BackgroundIndexLaunchState::ScopeBlocked);
    }
    launch_index_worker(config, project_root, force, mode, app)
}

/// 同步完成任务去重、检查点初始化并启动异步 worker。
/// 后继任务也走这里，避免 worker 内再次 await 自身形成递归 Future。
fn launch_index_worker(
    config: &AcemcpConfig,
    project_root: &str,
    force: bool,
    mode: IndexJobMode,
    app: Option<AppHandle>,
) -> anyhow::Result<BackgroundIndexLaunchState> {
    let normalized_root = normalize_project_path(
        &PathBuf::from(project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root))
            .to_string_lossy(),
    );
    let scope_hash = require_index_scope_hash(config)?;
    let initial_state = get_initial_index_state(project_root);
    let existing_job = jobs::get_job(&normalized_root);

    if !force {
        match initial_state {
            InitialIndexState::Synced
                if existing_job
                    .as_ref()
                    .map(|job| !job.is_resumable())
                    .unwrap_or(true) =>
            {
                return Ok(BackgroundIndexLaunchState::Skipped);
            }
            InitialIndexState::Indexing if existing_job.is_none() => {}
            InitialIndexState::Missing
            | InitialIndexState::Idle
            | InitialIndexState::Failed
            | InitialIndexState::Synced
            | InitialIndexState::Indexing => {}
        }
    }

    {
        let mut inflight = auto_index_inflight().lock().unwrap();
        if !inflight.insert(normalized_root.clone()) {
            if force {
                request_followup_index(&normalized_root, mode);
            }
            return Ok(BackgroundIndexLaunchState::AlreadyRunning);
        }
    }

    // 中文说明：项目 lease 是跨进程互斥边界，进程退出后由操作系统自动释放，旧检查点可被接管。
    let lease = match jobs::try_acquire_project_lease(&normalized_root) {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            auto_index_inflight().lock().unwrap().remove(&normalized_root);
            if force {
                request_followup_index(&normalized_root, mode);
            }
            return Ok(BackgroundIndexLaunchState::AlreadyRunning);
        }
        Err(error) => {
            auto_index_inflight().lock().unwrap().remove(&normalized_root);
            return Err(error);
        }
    };

    // 取得 lease 后重新读取，避免 GUI 与 MCP 同时更新清单时使用旧快照。
    let existing_job = jobs::get_job(&normalized_root);
    let job = match existing_job {
        Some(job)
            if job.is_resumable()
                && job.scope_hash == scope_hash
                && job.config_fingerprint == scope_hash
                && (job.mode == mode.as_str()
                    || (job.mode == IndexJobMode::Full.as_str()
                        && mode == IndexJobMode::Incremental)) => job,
        _ => match jobs::create_job(&normalized_root, mode.as_str(), &scope_hash, &scope_hash) {
            Ok(job) => job,
            Err(error) => {
                auto_index_inflight().lock().unwrap().remove(&normalized_root);
                return Err(error);
            }
        },
    };
    if let Some(app) = app.as_ref() {
        jobs::register_event_app(app);
    }
    let job_id = job.job_id.clone();
    let _ = jobs::update_job(
        &normalized_root,
        "queued",
        Some("后台任务已排队".to_string()),
        |job| job.status = JOB_QUEUED.to_string(),
    );
    let _ = update_project_status(project_root, |status| {
        status.status = IndexStatus::Indexing;
        status.progress = 0;
        status.last_error = None;
        status.last_failure_scope_hash = None;
    });

    let config_clone = config.clone();
    let project_root_clone = project_root.to_string();
    let normalized_root_clone = normalized_root.clone();
    tokio::spawn(async move {
        let lease = lease;
        log_important!(
            info,
            "后台索引任务启动: project_root={}, job_id={}",
            project_root_clone,
            job_id
        );
        let task_succeeded = match update_index_with_mode(
            &config_clone,
            &project_root_clone,
            mode,
        )
        .await
        {
            Ok(_) => {
                log_important!(info, "后台索引成功: project_root={}", project_root_clone);
                true
            }
            Err(error) => {
                let error_message = error.to_string();
                log_important!(
                    info,
                    "后台索引失败: project_root={}, error={}",
                    project_root_clone,
                    error_message
                );
                // 认证失败已经由上传循环记录了失败签名，不能在这里覆盖它。
                if !is_ace_auth_failure_error(&error_message) {
                    let should_record = jobs::get_job(&normalized_root_clone)
                        .map(|job| job.status != JOB_PAUSED && job.status != JOB_COMPLETED)
                        .unwrap_or(true);
                    if should_record {
                        let _ = jobs::update_job(
                            &normalized_root_clone,
                            "failed",
                            Some(error_message.clone()),
                            |job| {
                                if job.status != JOB_COMPLETED {
                                    // 网络/进程级失败保留为 paused，启动后可继续重试剩余批次。
                                    job.status = JOB_PAUSED.to_string();
                                    job.last_error = Some(error_message.clone());
                                }
                            },
                        );
                    }
                    let _ = update_project_status(&project_root_clone, |status| {
                        if status.status == IndexStatus::Indexing {
                            let resumable = jobs::get_job(&normalized_root_clone)
                                .map(|job| job.status == JOB_PAUSED)
                                .unwrap_or(false);
                            status.status = if resumable {
                                IndexStatus::Paused
                            } else {
                                IndexStatus::Failed
                            };
                            status.last_error = Some(error_message.clone());
                            status.last_failure_time = Some(chrono::Utc::now());
                        }
                    });
                }
                false
            }
        };

        {
            let mut inflight = auto_index_inflight().lock().unwrap();
            inflight.remove(&normalized_root_clone);
        }
        // 中文说明：当前 worker 已结束，先释放项目 lease，再尝试启动配置变更或后继任务。
        drop(lease);

        // 中文说明：配置可能在任务执行期间被保存；旧任务退出后立即接续一次新签名的全量任务。
        if let Ok(latest_config) = AcemcpTool::get_acemcp_config().await {
            if build_index_scope_hash(&latest_config) != build_index_scope_hash(&config_clone) {
                log_important!(
                    info,
                    "检测到索引任务执行期间 ACE 配置已变更，提交新签名全量任务: project_root={}",
                    normalized_root_clone
                );
                let _ = start_background_index_with_mode(
                    &latest_config,
                    &normalized_root_clone,
                    true,
                    IndexJobMode::Full,
                    None,
                )
                .await;
            } else if task_succeeded {
                // 当前任务收集文件后发生的新变更，合并为一轮后继任务，避免异步排队吞掉监听事件。
                let rerun_mode = jobs::get_job(&normalized_root_clone)
                    .and_then(|job| job.rerun_mode)
                    .map(|mode| IndexJobMode::from_str(&mode));
                if let Some(rerun_mode) = rerun_mode {
                    let _ = start_background_index_with_mode(
                        &latest_config,
                        &normalized_root_clone,
                        true,
                        rerun_mode,
                        None,
                    )
                    .await;
                }
            }
        }
    });

    Ok(BackgroundIndexLaunchState::Started)
}

/// 确保后台索引已启动（非阻塞）
/// 仅在项目未初始化或索引失败时启动后台索引任务
pub async fn ensure_initial_index_background(
    config: &AcemcpConfig,
    project_root: &str,
) -> anyhow::Result<()> {
    let project_status = get_project_status(project_root);
    if should_hold_on_auth_failure(config, project_root, &project_status) {
        log_important!(
            info,
            "跳过后台索引：检测到 Token 认证失败，需用户手动更新配置: project_root={}",
            project_root
        );
        return Ok(());
    }

    let state = get_initial_index_state(project_root);

    match state {
        InitialIndexState::Missing
        | InitialIndexState::Idle
        | InitialIndexState::Failed
        | InitialIndexState::Synced
        | InitialIndexState::Indexing => {
            // 启动函数会先核对 manifest；存在未完成任务时即使状态为 indexing 也会恢复。
            let _ = start_background_index(config, project_root, false).await?;
        }
    }
    Ok(())
}

/// MCP 进程启动时恢复清单中未完成的任务；已完成任务不会重复上传。
pub async fn resume_index_jobs() -> anyhow::Result<()> {
    let pending_jobs = jobs::resumable_jobs();
    if pending_jobs.is_empty() {
        return Ok(());
    }
    let config = AcemcpTool::get_acemcp_config().await?;
    let Some(current_scope_hash) = build_index_scope_hash(&config) else {
        log_important!(warn, "恢复 ACE 索引任务暂缓：当前配置缺少 base_url 或 token");
        return Ok(());
    };
    for job in pending_jobs {
        let status = get_project_status(&job.project_root);
        if should_hold_on_auth_failure(&config, &job.project_root, &status) {
            log_important!(
                info,
                "恢复索引任务暂缓：Token 认证失败，project_root={}, job_id={}",
                job.project_root,
                job.job_id
            );
            continue;
        }
        if current_scope_hash != job.scope_hash {
            // 中文说明：清单签名落后于当前 ACE 配置时交给全量 worker 接管；不先删除旧清单，避免与并行进程竞态。
            let _ = start_background_index_with_mode(
                &config,
                &job.project_root,
                true,
                IndexJobMode::Full,
                None,
            )
            .await?;
            continue;
        }
        let _ = start_background_index_with_mode(
            &config,
            &job.project_root,
            false,
            IndexJobMode::from_str(&job.mode),
            None,
        )
        .await?;
    }
    Ok(())
}

/// 文件监听只负责排队，上传由统一后台任务执行，确保同样具备批次断点与事件。
pub(crate) async fn enqueue_incremental_index(
    config: &AcemcpConfig,
    project_root: &str,
) -> anyhow::Result<()> {
    let _ = start_background_index_with_mode(
        config,
        project_root,
        true,
        IndexJobMode::Incremental,
        None,
    )
    .await?;
    Ok(())
}

// ---------------- 整合 temp 逻辑：索引、上传、检索 ----------------

#[derive(Serialize, Deserialize, Clone)]
struct BlobItem {
    path: String,
    content: String,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct ProjectsFile(pub HashMap<String, Vec<String>>);

fn normalize_base_url(input: &str) -> String {
    let mut url = input.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        url = format!("http://{}", url);
    }
    while url.ends_with('/') {
        url.pop();
    }
    url
}

const ACE_AUTH_FAILURE_MESSAGE: &str =
    "ACE API 认证失败 (401)：Token 已失效或被封禁，请在设置中更新 Token";
const ACE_AUTH_FAILURE_SEARCH_MESSAGE: &str = "代码搜索失败：ACE API Token 已失效。请前往【设置 > MCP 工具 > 代码搜索工具】更新认证令牌后重试。";

fn is_ace_auth_failure_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("invalid token")
        || message.contains("认证失败")
}

fn has_ace_auth_failure(status: &ProjectIndexStatus) -> bool {
    status.status == IndexStatus::Failed
        && status
            .last_error
            .as_deref()
            .map(is_ace_auth_failure_error)
            .unwrap_or(false)
}

fn mark_project_auth_failure(project_root: &str, scope_hash: Option<&str>) {
    let failure_scope_hash = scope_hash.map(str::to_string);
    let _ = update_project_status(project_root, |status| {
        status.status = IndexStatus::Failed;
        status.last_error = Some(ACE_AUTH_FAILURE_MESSAGE.to_string());
        status.last_failure_time = Some(chrono::Utc::now());
        status.last_failure_scope_hash = failure_scope_hash.clone();
    });
}

fn should_hold_on_auth_failure(
    config: &AcemcpConfig,
    project_root: &str,
    status: &ProjectIndexStatus,
) -> bool {
    if !has_ace_auth_failure(status) {
        return false;
    }

    let current_scope_hash = build_index_scope_hash(config);
    match (
        status.last_failure_scope_hash.as_deref(),
        current_scope_hash.as_deref(),
    ) {
        (Some(failure_hash), Some(current_hash)) => failure_hash == current_hash,
        (Some(_), None) => false,
        (None, Some(current_hash)) => {
            // 兼容历史状态：首次识别到认证失败时补写失败签名，后续在用户更新 Token 后可自动恢复。
            let current_hash = current_hash.to_string();
            let _ = update_project_status(project_root, |status| {
                if has_ace_auth_failure(status) && status.last_failure_scope_hash.is_none() {
                    status.last_failure_scope_hash = Some(current_hash.clone());
                }
            });
            true
        }
        (None, None) => true,
    }
}

pub(crate) fn should_skip_auto_index_for_auth_failure(
    config: &AcemcpConfig,
    project_root: &str,
) -> bool {
    let status = get_project_status(project_root);
    should_hold_on_auth_failure(config, project_root, &status)
}

fn has_local_blob_names(project_root: &str) -> bool {
    let projects = load_projects_file();

    let normalized_root = normalize_project_path(
        &PathBuf::from(project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root))
            .to_string_lossy(),
    );

    projects
        .0
        .get(&normalized_root)
        .map(|blob_names| !blob_names.is_empty())
        .unwrap_or(false)
}

/// 规范化项目路径，去除 Windows 扩展路径前缀并统一使用正斜杠
///
/// Windows 的 `canonicalize()` 会返回 `//?/C:/...` 或 `\\?\C:\...` 格式的路径，
/// 这会导致前后端路径匹配失败。此函数确保路径格式统一。
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

async fn retry_request<F, Fut, T>(
    mut f: F,
    max_retries: usize,
    base_delay_secs: f64,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 0usize;
    let mut last_error_str: Option<String> = None;

    while attempt < max_retries {
        match f().await {
            Ok(v) => {
                if attempt > 0 {
                    log_debug!("请求在第{}次尝试后成功", attempt + 1);
                }
                return Ok(v);
            }
            Err(e) => {
                last_error_str = Some(e.to_string());
                attempt += 1;

                // 检查是否为可重试的错误
                let error_str = e.to_string();
                let is_retryable = error_str.contains("timeout")
                    || error_str.contains("connection")
                    || error_str.contains("network")
                    || error_str.contains("temporary");

                if attempt >= max_retries || !is_retryable {
                    log_debug!("请求失败，不再重试: {}", e);
                    return Err(e);
                }

                let delay = base_delay_secs * 2f64.powi((attempt as i32) - 1);
                let ms = (delay * 1000.0) as u64;
                log_debug!(
                    "请求失败，准备重试({}/{}), 等待 {}ms: {}",
                    attempt,
                    max_retries,
                    ms,
                    e
                );
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
        }
    }

    Err(last_error_str
        .and_then(|s| anyhow::anyhow!(s).into())
        .unwrap_or_else(|| anyhow::anyhow!("未知错误")))
}

pub(crate) fn home_projects_file() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = home.join(".acemcp").join("data");
    let _ = fs::create_dir_all(&data_dir);
    data_dir.join("projects.json")
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_string_pretty(value)?;
    let tmp_path = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    let backup_path = path.with_extension("json.bak");
    fs::write(&tmp_path, &data)?;
    // Windows 目标文件已存在时 rename 可能失败；先保留备份，确保突然退出仍可恢复。
    let had_original = path.exists();
    if had_original {
        let _ = fs::remove_file(&backup_path);
        if let Err(error) = fs::rename(path, &backup_path) {
            log::warn!("备份 ACE 索引状态失败，将退回直接写入: {}", error);
            fs::write(path, data)?;
            let _ = fs::remove_file(&tmp_path);
            return Ok(());
        }
    }
    if let Err(rename_error) = fs::rename(&tmp_path, path) {
        log::warn!("原子替换 ACE 索引状态失败，将尝试恢复备份: {}", rename_error);
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

fn load_projects_file() -> ProjectsFile {
    let path = home_projects_file();
    load_json_with_backup(&path)
}

fn save_projects_file(projects: &ProjectsFile) -> Result<()> {
    write_json_atomically(&home_projects_file(), projects)
}

fn load_json_with_backup<T>(path: &Path) -> T
where
    T: DeserializeOwned + Default,
{
    let backup_path = path.with_extension("json.bak");
    for candidate in [path, backup_path.as_path()] {
        let Ok(data) = fs::read_to_string(candidate) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str(&data) {
            return value;
        }
    }
    T::default()
}

/// 获取项目索引状态文件路径
fn home_projects_status_file() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = home.join(".acemcp").join("data");
    let _ = fs::create_dir_all(&data_dir);
    data_dir.join("projects_status.json")
}

/// 读取所有项目的索引状态
fn load_projects_status() -> ProjectsIndexStatus {
    let status_path = home_projects_status_file();
    log_debug!("📂 [load_projects_status] 状态文件路径: {:?}", status_path);

    let status = load_json_with_backup::<ProjectsIndexStatus>(&status_path);
    if status.projects.is_empty() && !status_path.exists() {
        log_debug!("📭 [load_projects_status] 状态文件不存在，返回空列表");
    } else {
        log_debug!(
            "✅ [load_projects_status] 状态清单读取完成，项目数: {}",
            status.projects.len()
        );
    }
    status
}

/// 保存所有项目的索引状态
fn save_projects_status(status: &ProjectsIndexStatus) -> Result<()> {
    let status_path = home_projects_status_file();
    write_json_atomically(&status_path, status)
}

/// 更新指定项目的索引状态
fn update_project_status<F>(project_root: &str, updater: F) -> Result<()>
where
    F: FnOnce(&mut ProjectIndexStatus),
{
    let _guard = projects_status_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("获取 projects_status.json 写入锁失败"))?;
    let mut all_status = load_projects_status();
    // 使用 normalize_project_path 去除 Windows 扩展路径前缀
    let normalized_root = normalize_project_path(
        &PathBuf::from(project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root))
            .to_string_lossy(),
    );

    let project_status = all_status
        .projects
        .entry(normalized_root.clone())
        .or_insert_with(|| {
            let mut status = ProjectIndexStatus::default();
            status.project_root = normalized_root;
            status
        });

    updater(project_status);
    save_projects_status(&all_status)?;
    Ok(())
}

/// 获取指定项目的索引状态
fn get_project_status(project_root: &str) -> ProjectIndexStatus {
    let all_status = load_projects_status();
    // 使用 normalize_project_path 去除 Windows 扩展路径前缀
    let normalized_root = normalize_project_path(
        &PathBuf::from(project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root))
            .to_string_lossy(),
    );

    let mut status = all_status
        .projects
        .get(&normalized_root)
        .cloned()
        .unwrap_or_else(|| {
            let mut status = ProjectIndexStatus::default();
            status.project_root = normalized_root;
            status
        });
    reconcile_project_status_with_job(&mut status);
    enrich_project_scope_state(&mut status);
    status
}

/// 读取状态时以任务清单检查点重新校正，避免进程突然退出后继续展示旧进度。
fn reconcile_project_status_with_job(status: &mut ProjectIndexStatus) {
    let Some(job) = jobs::get_job(&status.project_root) else {
        return;
    };
    status.job_id = Some(job.job_id.clone());
    status.total_batches = job.total_batches;
    status.completed_batches = job.completed_batches;
    status.job_updated_at = Some(job.updated_at.clone());
    if job.total_blobs > 0 {
        status.total_files = job.total_blobs;
        status.indexed_files = job.completed_blobs.min(job.total_blobs);
        status.pending_files = job.total_blobs.saturating_sub(status.indexed_files);
        status.progress = calculate_index_progress(status.indexed_files, status.total_files);
    }
    match job.status.as_str() {
        JOB_QUEUED | JOB_COLLECTING | JOB_UPLOADING => {
            status.status = IndexStatus::Indexing;
            status.last_error = job.last_error.clone();
        }
        JOB_PAUSED => {
            // 可恢复暂停保留当前断点，但不再显示为永久失败或持续旋转。
            status.status = IndexStatus::Paused;
            if status.last_failure_time.is_none() {
                status.last_failure_time = Some(job.updated_at.clone());
            }
            status.last_error = job.last_error.clone();
        }
        JOB_SCOPE_BLOCKED => {
            status.status = IndexStatus::Paused;
            status.last_error = job.last_error.clone();
        }
        JOB_FAILED => {
            status.status = IndexStatus::Failed;
            if status.last_failure_time.is_none() {
                status.last_failure_time = Some(job.updated_at.clone());
            }
            if let Some(job_error) = job.last_error.clone() {
                if is_ace_auth_failure_error(&job_error) {
                    status.last_failure_scope_hash = Some(job.scope_hash.clone());
                }
                status.last_error = Some(job_error);
            }
        }
        JOB_COMPLETED => {
            if status.indexed_files >= status.total_files {
                status.status = IndexStatus::Synced;
                status.progress = 100;
                status.pending_files = 0;
                status.last_error = None;
                if status.last_success_time.is_none() {
                    status.last_success_time = Some(job.updated_at.clone());
                }
            }
        }
        _ => {}
    }
}

/// 读取文件内容，支持多种编码检测
/// 尝试的编码顺序：utf-8, gbk (包含 gb2312), windows-1252 (包含 latin-1)
/// 如果都失败，则使用 utf-8 with errors='ignore'
fn read_file_with_encoding(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return None;
    }

    // 尝试 utf-8
    let (decoded, _, had_errors) = UTF_8.decode(&buf);
    if !had_errors {
        return Some(decoded.into_owned());
    }

    // 尝试 gbk
    let (decoded, _, had_errors) = GBK.decode(&buf);
    if !had_errors {
        log_debug!("成功使用 GBK 编码读取文件: {:?}", path);
        return Some(decoded.into_owned());
    }

    // 尝试 gb2312 (GBK 是 GB2312 的超集，可以处理 GB2312 编码)
    // encoding_rs 中没有单独的 GB2312，使用 GBK 代替
    // GBK 已经在上一步尝试过了，这里跳过

    // 尝试 latin-1 (WINDOWS_1252 是 ISO-8859-1 的超集，可以处理大部分 latin-1 编码)
    let (decoded, _, had_errors) = WINDOWS_1252.decode(&buf);
    if !had_errors {
        log_debug!("成功使用 WINDOWS_1252 编码读取文件: {:?}", path);
        return Some(decoded.into_owned());
    }

    // 如果所有编码都失败，使用 utf-8 with errors='ignore' (lossy 解码)
    let (decoded, _, _) = UTF_8.decode(&buf);
    log_debug!("使用 UTF-8 (lossy) 读取文件，部分字符可能丢失: {:?}", path);
    Some(decoded.into_owned())
}

fn sha256_hex(path: &str, content: &str) -> String {
    let mut ctx = ShaContext::new(&SHA256);
    // 先更新路径的哈希，再更新内容的哈希，与Python版本保持一致
    ctx.update(path.as_bytes());
    ctx.update(content.as_bytes());
    let digest = ctx.finish();
    hex::encode(digest.as_ref())
}

/// 分割文件内容为多个 blob（如果超过最大行数）
/// 与 Python 版本保持一致：chunk 索引从 1 开始
fn split_content(path: &str, content: &str, max_lines: usize) -> Vec<BlobItem> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let total_lines = lines.len();

    // 如果文件在限制内，返回单个 blob
    if total_lines <= max_lines {
        return vec![BlobItem {
            path: path.to_string(),
            content: content.to_string(),
        }];
    }

    // 计算需要的 chunk 数量
    let num_chunks = (total_lines + max_lines - 1) / max_lines;
    let mut blobs = Vec::new();

    // 按 chunk 索引分割（从 0 开始，但显示时从 1 开始）
    for chunk_idx in 0..num_chunks {
        let start_line = chunk_idx * max_lines;
        let end_line = usize::min(start_line + max_lines, total_lines);
        let chunk_lines = &lines[start_line..end_line];
        let chunk_content = chunk_lines.join("");

        // chunk 编号从 1 开始（与 Python 版本保持一致）
        let chunk_path = format!("{}#chunk{}of{}", path, chunk_idx + 1, num_chunks);
        blobs.push(BlobItem {
            path: chunk_path,
            content: chunk_content,
        });
    }

    blobs
}

// 去除 blob 路径中的 chunk 后缀，恢复文件级路径
fn strip_chunk_suffix(path: &str) -> &str {
    path.split("#chunk").next().unwrap_or(path)
}

/// 构建排除模式的 GlobSet
pub(crate) fn build_exclude_globset(exclude_patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in exclude_patterns {
        // 尝试将模式转换为 Glob
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        } else {
            log_debug!("无效的排除模式，跳过: {}", pattern);
        }
    }
    builder
        .build()
        .map_err(|e| anyhow::anyhow!("构建排除模式失败: {}", e))
}

/// 检查路径是否应该被排除
/// 使用 globset 进行完整的 fnmatch 模式匹配（与 Python 版本保持一致）
/// Python 版本使用 fnmatch.fnmatch 检查路径的各个部分和完整路径
pub(crate) fn should_exclude(
    path: &Path,
    root: &Path,
    exclude_globset: Option<&GlobSet>,
) -> bool {
    if exclude_globset.is_none() {
        return false;
    }
    let globset = exclude_globset.unwrap();

    // 获取相对路径
    let rel = match path.strip_prefix(root) {
        Ok(rel) => rel,
        Err(_) => path,
    };

    // 转换为使用正斜杠的字符串（用于匹配）
    let rel_forward = rel.to_string_lossy().replace('\\', "/");

    // 检查完整相对路径（与 Python 版本的 fnmatch(path_str, pattern) 一致）
    if globset.is_match(&rel_forward) {
        return true;
    }

    // 检查路径的各个部分（与 Python 版本的 fnmatch(part, pattern) 一致）
    for part in rel.iter() {
        if let Some(part_str) = part.to_str() {
            if globset.is_match(part_str) {
                return true;
            }
        }
    }

    false
}

fn build_gitignore(root: &Path) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    let gi_path = root.join(".gitignore");
    if gi_path.exists() {
        if builder.add(gi_path).is_some() {
            return None;
        }
        return match builder.build() {
            Ok(gi) => Some(gi),
            Err(_) => None,
        };
    }
    None
}

fn collect_blobs(
    root: &str,
    text_exts: &[String],
    exclude_patterns: &[String],
    max_lines_per_blob: usize,
) -> anyhow::Result<Vec<BlobItem>> {
    let root_path = PathBuf::from(root);
    if !root_path.exists() {
        anyhow::bail!("项目根目录不存在: {}", root);
    }

    log_important!(
        info,
        "开始收集代码文件: 根目录={}, 扩展名={:?}, 排除模式={:?}",
        root,
        text_exts,
        exclude_patterns
    );

    // 构建排除模式的 GlobSet
    let exclude_globset = if exclude_patterns.is_empty() {
        None
    } else {
        match build_exclude_globset(exclude_patterns) {
            Ok(gs) => Some(gs),
            Err(e) => {
                log_debug!("构建排除模式失败，将使用简单匹配: {}", e);
                None
            }
        }
    };

    let mut out = Vec::new();
    let gitignore = build_gitignore(&root_path);
    let mut dirs_stack = vec![root_path.clone()];
    let mut scanned_files = 0;
    let mut indexed_files = 0;
    let mut excluded_count = 0;

    while let Some(dir) = dirs_stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // 中文说明：不跟随符号链接或目录联接，保证实际收集范围与预检一致。
            if file_type.is_symlink() {
                continue;
            }

            // 检查 .gitignore
            if let Some(gi) = &gitignore {
                if gi
                    .matched_path_or_any_parents(&p, file_type.is_dir())
                    .is_ignore()
                {
                    continue;
                }
            }

            // 检查排除模式
            if file_type.is_dir() {
                if should_exclude(&p, &root_path, exclude_globset.as_ref()) {
                    excluded_count += 1;
                    continue;
                }
                dirs_stack.push(p);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            scanned_files += 1;
            if should_exclude(&p, &root_path, exclude_globset.as_ref()) {
                excluded_count += 1;
                log_debug!("排除文件: {:?}", p);
                continue;
            }

            // 检查文件扩展名
            let ext_ok = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|e| {
                    let dot = format!(".{}", e).to_lowercase();
                    text_exts.iter().any(|te| te.eq_ignore_ascii_case(&dot))
                })
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }

            // 读取文件内容（使用多编码支持）
            let rel = p
                .strip_prefix(&root_path)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if let Some(content) = read_file_with_encoding(&p) {
                let parts = split_content(&rel, &content, max_lines_per_blob);
                let blob_count = parts.len();
                indexed_files += 1;
                out.extend(parts);
                log_debug!(
                    "索引文件: path={}, content_length={}, blobs={}",
                    rel,
                    content.len(),
                    blob_count
                );
            } else {
                log_debug!("无法读取文件: {:?}", p);
            }
        }
    }

    log_important!(
        info,
        "文件收集完成: 扫描文件数={}, 索引文件数={}, 生成blobs数={}, 排除文件/目录数={}",
        scanned_files,
        indexed_files,
        out.len(),
        excluded_count
    );
    Ok(out)
}

/// 收集项目内所有可索引文件的索引状态
///
/// 为避免引入新的持久化结构，这里通过重新扫描文件并复用与索引阶段相同的
/// 路径规范化与分块逻辑，基于现有的 blob 哈希集合判断文件是否“已完全索引”。
fn collect_file_statuses(
    root: &str,
    text_exts: &[String],
    exclude_patterns: &[String],
    max_lines_per_blob: usize,
    existing_blob_names: &HashSet<String>,
) -> anyhow::Result<Vec<FileIndexStatus>> {
    let root_path = PathBuf::from(root);
    if !root_path.exists() {
        anyhow::bail!("项目根目录不存在: {}", root);
    }

    // 构建排除模式的 GlobSet
    let exclude_globset = if exclude_patterns.is_empty() {
        None
    } else {
        match build_exclude_globset(exclude_patterns) {
            Ok(gs) => Some(gs),
            Err(e) => {
                log_debug!("构建排除模式失败，将使用简单匹配: {}", e);
                None
            }
        }
    };

    let gitignore = build_gitignore(&root_path);
    let mut dirs_stack = vec![root_path.clone()];
    let mut files_status = Vec::new();

    while let Some(dir) = dirs_stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }

            // .gitignore 过滤
            if let Some(gi) = &gitignore {
                if gi
                    .matched_path_or_any_parents(&p, file_type.is_dir())
                    .is_ignore()
                {
                    continue;
                }
            }

            if file_type.is_dir() {
                if should_exclude(&p, &root_path, exclude_globset.as_ref()) {
                    continue;
                }
                dirs_stack.push(p);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            if should_exclude(&p, &root_path, exclude_globset.as_ref()) {
                continue;
            }

            // 扩展名过滤
            let ext_ok = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|e| {
                    let dot = format!(".{}", e).to_lowercase();
                    text_exts.iter().any(|te| te.eq_ignore_ascii_case(&dot))
                })
                .unwrap_or(false);

            if !ext_ok {
                continue;
            }

            let rel = p
                .strip_prefix(&root_path)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");

            // 读取文件内容并根据分块结果计算 blob 哈希
            if let Some(content) = read_file_with_encoding(&p) {
                let blobs = split_content(&rel, &content, max_lines_per_blob);
                if blobs.is_empty() {
                    continue;
                }

                let mut all_indexed = true;
                for blob in &blobs {
                    let hash = sha256_hex(&blob.path, &blob.content);
                    if !existing_blob_names.contains(&hash) {
                        all_indexed = false;
                        break;
                    }
                }

                let status = if all_indexed {
                    FileIndexStatusKind::Indexed
                } else {
                    FileIndexStatusKind::Pending
                };

                files_status.push(FileIndexStatus {
                    path: rel.clone(),
                    status,
                });
            } else {
                // 无法读取内容时，保守地标记为 Pending，避免静默丢失
                files_status.push(FileIndexStatus {
                    path: rel.clone(),
                    status: FileIndexStatusKind::Pending,
                });
            }
        }
    }

    Ok(files_status)
}

fn calculate_index_progress(completed: usize, total: usize) -> u8 {
    if total == 0 {
        return 0;
    }
    ((completed.saturating_mul(100) / total).min(100)) as u8
}

fn ensure_index_job(
    normalized_root: &str,
    mode: IndexJobMode,
    scope_hash: &str,
) -> anyhow::Result<IndexJob> {
    if let Some(job) = jobs::get_job(normalized_root) {
        if job.is_resumable()
            && job.mode == mode.as_str()
            && job.scope_hash == scope_hash
            && job.config_fingerprint == scope_hash
        {
            return Ok(job);
        }
    }
    jobs::create_job(
        normalized_root,
        mode.as_str(),
        scope_hash,
        scope_hash,
    )
}

fn persist_confirmed_blob_names(
    normalized_root: &str,
    existing_hashes: &HashSet<String>,
    uploaded_names: &[String],
) -> anyhow::Result<Vec<String>> {
    let _guard = projects_file_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("获取 projects.json 写入锁失败"))?;
    // 每批写入前重新加载，避免并行子项目拿旧快照覆盖其他项目的进度。
    let mut projects = load_projects_file();
    let mut confirmed = existing_hashes.iter().cloned().collect::<Vec<_>>();
    confirmed.extend(uploaded_names.iter().cloned());
    confirmed.sort();
    confirmed.dedup();
    projects
        .0
        .insert(normalized_root.to_string(), confirmed.clone());
    save_projects_file(&projects)?;
    Ok(confirmed)
}

fn mark_index_job_error(
    project_root_path: &str,
    normalized_root: &str,
    message: &str,
    failed_batch: Option<usize>,
    failed_blobs: usize,
    auth_scope_hash: Option<&str>,
) {
    let error_message = message.to_string();
    let resumable_failure = failed_batch.is_some() && auth_scope_hash.is_none();
    let event_type = if resumable_failure { "paused" } else { "failed" };
    let _ = jobs::update_job(
        normalized_root,
        event_type,
        Some(error_message.clone()),
        |job| {
            job.status = if resumable_failure {
                JOB_PAUSED.to_string()
            } else {
                JOB_FAILED.to_string()
            };
            job.last_error = Some(error_message.clone());
            if let Some(batch) = failed_batch {
                if !job.failed_batches.contains(&batch) {
                    job.failed_batches.push(batch);
                }
            }
        },
    );
    let progress_job = jobs::get_job(normalized_root);
    let _ = update_project_status(project_root_path, |status| {
        let resumable = progress_job
            .as_ref()
            .map(|job| job.status == JOB_PAUSED)
            .unwrap_or(false);
        status.status = if resumable {
            IndexStatus::Paused
        } else {
            IndexStatus::Failed
        };
        if let Some(job) = &progress_job {
            status.total_files = job.total_blobs;
            status.indexed_files = job.completed_blobs.min(job.total_blobs);
            status.pending_files = job.total_blobs.saturating_sub(status.indexed_files);
            status.progress = calculate_index_progress(status.indexed_files, status.total_files);
        }
        status.failed_files = failed_blobs;
        status.last_error = Some(error_message.clone());
        status.last_failure_time = Some(chrono::Utc::now());
        status.last_failure_scope_hash = auth_scope_hash.map(str::to_string);
    });
}

/// 只执行索引更新，不进行搜索。默认按增量任务执行。
pub(crate) async fn update_index(
    config: &AcemcpConfig,
    project_root_path: &str,
) -> anyhow::Result<Vec<String>> {
    update_index_with_mode(config, project_root_path, IndexJobMode::Incremental).await
}

async fn update_index_with_mode(
    config: &AcemcpConfig,
    project_root_path: &str,
    mode: IndexJobMode,
) -> anyhow::Result<Vec<String>> {
    let base_url = config
        .base_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("未配置 base_url"))?;
    let has_scheme = base_url.starts_with("http://") || base_url.starts_with("https://");
    let has_host = base_url.trim().len() > "https://".len();
    if !has_scheme || !has_host {
        anyhow::bail!("无效的 base_url，请填写完整的 http(s)://host[:port] 格式");
    }
    let token = config
        .token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("未配置 token"))?;
    let current_scope_hash = require_index_scope_hash(config)?;
    let batch_size = (config.batch_size.unwrap_or(10) as usize).max(1);
    let max_lines = (config.max_lines_per_blob.unwrap_or(800) as usize).max(1);
    let text_exts = config.text_extensions.clone().unwrap_or_default();
    let exclude_patterns = effective_exclude_patterns(config.exclude_patterns.as_deref());
    let normalized_root = normalize_project_path(
        &PathBuf::from(project_root_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root_path))
            .to_string_lossy(),
    );
    ensure_index_job(&normalized_root, mode, &current_scope_hash)?;

    let job = jobs::update_job(&normalized_root, "collecting", None, |job| {
        job.status = JOB_COLLECTING.to_string();
        job.last_error = None;
        job.failed_batches.clear();
    })?
    .ok_or_else(|| anyhow::anyhow!("ACE 索引任务检查点不存在"))?;
    let _ = update_project_status(project_root_path, |status| {
        status.status = IndexStatus::Indexing;
        status.progress = 0;
        status.last_error = None;
        status.last_failure_scope_hash = None;
        status.failed_files = 0;
    });

    log_important!(info, "=== 开始索引代码库 ===");
    log_important!(
        info,
        "ACE索引任务: job_id={}, mode={}, project_root={}, batch_size={}, max_lines_per_blob={}",
        job.job_id,
        mode.as_str(),
        normalized_root,
        batch_size,
        max_lines
    );

    let blobs = match collect_blobs(project_root_path, &text_exts, &exclude_patterns, max_lines) {
        Ok(blobs) if !blobs.is_empty() => blobs,
        Ok(_) => {
            let message = "未在项目中找到可索引的文本文件";
            mark_index_job_error(
                project_root_path,
                &normalized_root,
                message,
                None,
                0,
                None,
            );
            anyhow::bail!(message);
        }
        Err(error) => {
            let message = format!("收集索引文件失败: {}", error);
            mark_index_job_error(
                project_root_path,
                &normalized_root,
                &message,
                None,
                0,
                None,
            );
            return Err(error);
        }
    };

    // 固定排序保证进程重启后重新分批时仍能稳定核对断点。
    let mut blob_entries = blobs
        .into_iter()
        .map(|blob| (sha256_hex(&blob.path, &blob.content), blob))
        .collect::<Vec<_>>();
    blob_entries.sort_by(|left, right| left.0.cmp(&right.0));
    let total_blobs = blob_entries.len();
    let all_blob_hashes = blob_entries
        .iter()
        .map(|(hash, _)| hash.clone())
        .collect::<HashSet<_>>();

    let projects = load_projects_file();
    let mut existing_blob_names = projects
        .0
        .get(&normalized_root)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let project_status = get_project_status(project_root_path);
    let scope_changed = mode == IndexJobMode::Full
        || is_index_scope_stale(
            &project_status,
            Some(current_scope_hash.as_str()),
            !existing_blob_names.is_empty(),
        );
    if scope_changed {
        log_important!(
            info,
            "检测到 ACE 索引空间或索引参数已变更，将按全量重建处理: project_root={}",
            normalized_root
        );
        existing_blob_names.clear();
    }

    let checkpoint_hashes = job
        .completed_blob_hashes
        .iter()
        .filter(|hash| all_blob_hashes.contains(*hash))
        .cloned()
        .collect::<HashSet<_>>();
    let checkpoint_names = job
        .uploaded_blob_names
        .iter()
        .filter(|name| all_blob_hashes.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    let existing_hashes = all_blob_hashes
        .intersection(&existing_blob_names)
        .cloned()
        .collect::<HashSet<_>>();
    let mut completed_hashes = existing_hashes.clone();
    completed_hashes.extend(checkpoint_hashes.iter().cloned());
    // 中文说明：原集合直接收缩并移动，避免百万级文件内容被完整 clone 两次。
    blob_entries.retain(|(hash, _)| !completed_hashes.contains(hash));
    let new_entries = blob_entries;
    let max_confirmed_batch_count = if checkpoint_hashes.is_empty() {
        0
    } else {
        (checkpoint_hashes.len() + batch_size - 1) / batch_size
    };
    let completed_batch_count = job.completed_batches.min(max_confirmed_batch_count);
    let remaining_batches = (new_entries.len() + batch_size - 1) / batch_size;
    let total_batches = completed_batch_count.saturating_add(remaining_batches);

    jobs::update_job(&normalized_root, "uploading", None, |job| {
        job.status = JOB_UPLOADING.to_string();
        job.total_blobs = total_blobs;
        job.completed_blobs = completed_hashes.len();
        job.completed_blob_hashes = checkpoint_hashes.iter().cloned().collect();
        job.uploaded_blob_names = checkpoint_names.clone();
        job.total_batches = total_batches;
        job.completed_batches = completed_batch_count;
        job.last_error = None;
    })?
    .ok_or_else(|| anyhow::anyhow!("ACE 索引任务检查点不存在"))?;
    let _ = update_project_status(project_root_path, |status| {
        status.status = IndexStatus::Indexing;
        status.total_files = total_blobs;
        status.indexed_files = completed_hashes.len();
        status.pending_files = total_blobs.saturating_sub(completed_hashes.len());
        status.failed_files = 0;
        status.progress = calculate_index_progress(status.indexed_files, status.total_files);
    });

    log_important!(
        info,
        "ACE断点核对: total_blobs={}, projects_confirmed={}, checkpoint_confirmed={}, remaining={}, progress={}％",
        total_blobs,
        existing_hashes.len(),
        completed_hashes.len().saturating_sub(existing_hashes.len()),
        new_entries.len(),
        calculate_index_progress(completed_hashes.len(), total_blobs)
    );

    let client = create_acemcp_client(config)?;
    let mut uploaded_this_run = Vec::new();
    for (batch_offset, batch_entries) in new_entries.chunks(batch_size).enumerate() {
        let batch_number = completed_batch_count + batch_offset + 1;
        let batch = batch_entries
            .iter()
            .map(|(_, blob)| blob.clone())
            .collect::<Vec<_>>();
        let batch_hashes = batch_entries
            .iter()
            .map(|(hash, _)| hash.clone())
            .collect::<Vec<_>>();
        let url = format!("{}/batch-upload", base_url);
        log_important!(
            info,
            "上传批次 {}/{}: project_root={}, blobs={}",
            batch_number,
            total_batches,
            normalized_root,
            batch.len()
        );
        let payload = serde_json::json!({"blobs": &batch});
        let response = retry_request(
            || async {
                let response = client
                    .post(&url)
                    .header(AUTHORIZATION, format!("Bearer {}", token))
                    .header(CONTENT_TYPE, "application/json")
                    .json(&payload)
                    .send()
                    .await?;
                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    anyhow::bail!("HTTP {} {}", status, body);
                }
                Ok(response.json::<serde_json::Value>().await?)
            },
            3,
            1.0,
        )
        .await;

        let returned_names = match response {
            Ok(value) => value
                .get("blob_names")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .filter(|names| !names.is_empty()),
            Err(error) => {
                let error_message = error.to_string();
                let auth_failure = is_ace_auth_failure_error(&error_message);
                let display_message = if auth_failure {
                    ACE_AUTH_FAILURE_MESSAGE.to_string()
                } else {
                    format!("批次 {} 上传失败: {}", batch_number, error_message)
                };
                mark_index_job_error(
                    project_root_path,
                    &normalized_root,
                    &display_message,
                    Some(batch_number),
                    batch.len(),
                    auth_failure.then_some(current_scope_hash.as_str()),
                );
                return Err(anyhow::anyhow!(display_message));
            }
        };

        let Some(returned_names) = returned_names else {
            let message = format!("批次 {} 响应中缺少可用的 blob_names", batch_number);
            mark_index_job_error(
                project_root_path,
                &normalized_root,
                &message,
                Some(batch_number),
                batch.len(),
                None,
            );
            anyhow::bail!(message);
        };

        // ACE 返回值在既有协议中就是本地 blob 哈希；逐项核对后只确认真实命中的部分。
        let returned_name_set = returned_names.into_iter().collect::<HashSet<_>>();
        let confirmed_batch_hashes = batch_hashes
            .iter()
            .filter(|hash| returned_name_set.contains(*hash))
            .cloned()
            .collect::<Vec<_>>();
        if confirmed_batch_hashes.is_empty() {
            let message = format!("批次 {} 返回的 blob_names 与本批哈希不匹配", batch_number);
            mark_index_job_error(
                project_root_path,
                &normalized_root,
                &message,
                Some(batch_number),
                batch.len(),
                None,
            );
            anyhow::bail!(message);
        }
        let confirmed_batch_set = confirmed_batch_hashes
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        uploaded_this_run.extend(
            batch_entries
                .iter()
                .filter(|(hash, _)| confirmed_batch_set.contains(hash))
                .map(|(_, blob)| blob.clone()),
        );
        let missing_blobs = batch_hashes
            .len()
            .saturating_sub(confirmed_batch_hashes.len());
        let batch_fully_confirmed = missing_blobs == 0;
        let updated_job = jobs::update_job(
            &normalized_root,
            if batch_fully_confirmed {
                "batch_completed"
            } else {
                "batch_partial"
            },
            Some(format!(
                "批次 {} 已确认 {} / {} 个 blobs",
                batch_number,
                confirmed_batch_hashes.len(),
                batch_hashes.len()
            )),
            |job| {
                job.status = JOB_UPLOADING.to_string();
                if batch_fully_confirmed {
                    job.completed_batches = batch_number;
                }
                job.completed_blob_hashes
                    .extend(confirmed_batch_hashes.clone());
                job.completed_blob_hashes.sort();
                job.completed_blob_hashes.dedup();
                job.uploaded_blob_names
                    .extend(confirmed_batch_hashes.clone());
                job.uploaded_blob_names.sort();
                job.uploaded_blob_names.dedup();
                let mut confirmed_hashes = existing_hashes.clone();
                confirmed_hashes.extend(job.completed_blob_hashes.iter().cloned());
                job.completed_blobs = confirmed_hashes.len().min(job.total_blobs);
                if batch_fully_confirmed {
                    job.failed_batches.retain(|batch| *batch != batch_number);
                } else if !job.failed_batches.contains(&batch_number) {
                    job.failed_batches.push(batch_number);
                }
                job.last_error = None;
            },
        )?
        .ok_or_else(|| anyhow::anyhow!("ACE 索引任务检查点不存在"))?;

        // 先写任务断点，再把已确认 blob 投影到 projects.json；任一步失败都不会误报完成。
        persist_confirmed_blob_names(
            &normalized_root,
            &existing_hashes,
            &updated_job.uploaded_blob_names,
        )?;
        let _ = update_project_status(project_root_path, |status| {
            status.status = IndexStatus::Indexing;
            status.total_files = updated_job.total_blobs;
            status.indexed_files = updated_job.completed_blobs;
            status.pending_files = status.total_files.saturating_sub(status.indexed_files);
            status.failed_files = 0;
            status.progress = calculate_index_progress(status.indexed_files, status.total_files);
            status.last_error = None;
        });
        if !batch_fully_confirmed {
            let message = format!(
                "批次 {} 仅确认 {} / {} 个 blobs，将从缺失部分继续",
                batch_number,
                confirmed_batch_hashes.len(),
                batch_hashes.len()
            );
            mark_index_job_error(
                project_root_path,
                &normalized_root,
                &message,
                Some(batch_number),
                missing_blobs,
                None,
            );
            anyhow::bail!(message);
        }
    }

    let final_job = jobs::get_job(&normalized_root)
        .ok_or_else(|| anyhow::anyhow!("ACE 索引任务在完成前丢失"))?;
    if final_job.completed_blobs != total_blobs {
        let message = format!(
            "索引检查点核对失败: 已确认 {} / 总计 {}",
            final_job.completed_blobs,
            total_blobs
        );
        mark_index_job_error(
            project_root_path,
            &normalized_root,
            &message,
            None,
            total_blobs.saturating_sub(final_job.completed_blobs),
            None,
        );
        anyhow::bail!(message);
    }

    let blob_names = persist_confirmed_blob_names(
        &normalized_root,
        &existing_hashes,
        &final_job.uploaded_blob_names,
    )?;
    if blob_names.is_empty() {
        let message = "索引后未找到 blobs";
        mark_index_job_error(
            project_root_path,
            &normalized_root,
            message,
            None,
            0,
            None,
        );
        anyhow::bail!(message);
    }

    let is_first_success = get_project_status(project_root_path)
        .last_success_time
        .is_none();
    let mut recent_files = uploaded_this_run
        .iter()
        .map(|blob| strip_chunk_suffix(&blob.path).to_string())
        .collect::<Vec<_>>();
    recent_files.sort();
    recent_files.dedup();
    recent_files.truncate(5);

    jobs::update_job(
        &normalized_root,
        "completed",
        Some(format!("索引完成，共 {} 个 blobs", total_blobs)),
        |job| {
            job.status = JOB_COMPLETED.to_string();
            job.completed_blobs = total_blobs;
            job.last_error = None;
            job.failed_batches.clear();
        },
    )?
    .ok_or_else(|| anyhow::anyhow!("ACE 索引任务在完成时丢失"))?;
    let _ = update_project_status(project_root_path, |status| {
        status.status = IndexStatus::Synced;
        status.progress = 100;
        status.total_files = total_blobs;
        status.indexed_files = total_blobs;
        status.pending_files = 0;
        status.failed_files = 0;
        status.last_success_time = Some(chrono::Utc::now());
        status.last_error = None;
        status.last_failure_scope_hash = None;
        status.index_scope_hash = Some(current_scope_hash.clone());
        status.is_stale = false;
        status.stale_reason = None;
        if !recent_files.is_empty() {
            status.recent_indexed_files = recent_files;
        }
    });

    if is_first_success {
        let _ = write_index_memory_to_ji(project_root_path, config);
    }
    log_important!(info, "索引更新完成，共 {} 个 blobs", blob_names.len());
    Ok(blob_names)
}

/// 将索引配置信息写入 ji（记忆）工具
fn write_index_memory_to_ji(project_root_path: &str, config: &AcemcpConfig) {
    use super::super::memory::MemoryCategory;
    use super::super::memory::MemoryManager;

    // 创建记忆管理器
    let mut manager = match MemoryManager::new(project_root_path) {
        Ok(m) => m,
        Err(e) => {
            log_debug!("创建记忆管理器失败（不影响索引）: {}", e);
            return;
        }
    };

    // 构建记忆内容
    let text_exts = config.text_extensions.clone().unwrap_or_default();
    let exclude_patterns = effective_exclude_patterns(config.exclude_patterns.as_deref());
    let batch_size = config.batch_size.unwrap_or(10);
    let max_lines = config.max_lines_per_blob.unwrap_or(800);

    let memory_content = format!(
        "acemcp 代码索引已启用 - 配置摘要: 文件扩展名={:?}, 排除模式={:?}, 批次大小={}, 最大行数/块={}",
        text_exts, exclude_patterns, batch_size, max_lines
    );

    // 写入记忆（add_memory 现在返回 Option<String>）
    match manager.add_memory(&memory_content, MemoryCategory::Context) {
        Ok(Some(id)) => {
            log_important!(info, "已将索引配置写入 ji 记忆: id={}", id);
        }
        Ok(None) => {
            log_debug!("索引配置记忆已存在相似内容，未重复添加");
        }
        Err(e) => {
            log_debug!("写入 ji 记忆失败（不影响索引）: {}", e);
        }
    }
}

/// 执行搜索
/// 若检测到索引缺失或 ACE 配置已变更，则改为返回后台索引提示
async fn search_only(
    config: &AcemcpConfig,
    project_root_path: &str,
    query: &str,
) -> anyhow::Result<String> {
    let base_url = config
        .base_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("未配置 base_url"))?;
    let token = config
        .token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("未配置 token"))?;
    let current_scope_hash = require_index_scope_hash(config)?;

    // 从 projects.json 读取已有的 blob 名称
    let projects = load_projects_file();

    // 使用 normalize_project_path 去除 Windows 扩展路径前缀
    let normalized_root = normalize_project_path(
        &PathBuf::from(project_root_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(project_root_path))
            .to_string_lossy(),
    );

    let blob_names = projects
        .0
        .get(&normalized_root)
        .cloned()
        .unwrap_or_default();
    let project_status = get_project_status(project_root_path);
    if should_hold_on_auth_failure(config, project_root_path, &project_status) {
        anyhow::bail!(ACE_AUTH_FAILURE_SEARCH_MESSAGE);
    }
    let scope_changed = is_index_scope_stale(
        &project_status,
        Some(current_scope_hash.as_str()),
        !blob_names.is_empty(),
    );

    if scope_changed {
        let launch_state = start_background_index_with_mode(
            config,
            project_root_path,
            true,
            IndexJobMode::Full,
            None,
        )
        .await?;
        let message = match launch_state {
            BackgroundIndexLaunchState::Started => {
                "检测到 API 配置已变更，当前项目索引已在后台重建，请稍后重试。"
            }
            BackgroundIndexLaunchState::AlreadyRunning => {
                "检测到 API 配置已变更，当前项目索引正在后台重建，请稍后重试。"
            }
            BackgroundIndexLaunchState::Skipped => "检测到 API 配置已变更，请稍后重试。",
            BackgroundIndexLaunchState::ScopeBlocked => {
                "项目索引范围存在风险，已暂停后台重建，请在等一下窗口中确认。"
            }
        };
        return Ok(message.to_string());
    }

    if blob_names.is_empty() {
        let launch_state = start_background_index(config, project_root_path, true).await?;
        let message = match launch_state {
            BackgroundIndexLaunchState::Started => {
                "当前项目尚未建立索引，已在后台启动索引，请稍后重试。"
            }
            BackgroundIndexLaunchState::AlreadyRunning => "当前项目正在后台索引中，请稍后重试。",
            BackgroundIndexLaunchState::Skipped => "当前项目索引尚未就绪，请稍后重试。",
            BackgroundIndexLaunchState::ScopeBlocked => {
                "项目索引范围存在风险，已暂停后台索引，请在等一下窗口中确认。"
            }
        };
        return Ok(message.to_string());
    }

    // 发起检索
    log_important!(info, "=== 开始代码检索（仅搜索模式） ===");
    let search_url = format!("{}/agents/codebase-retrieval", base_url);
    log_important!(
        info,
        "检索请求: url={}, 使用blobs数量={}, 查询内容={}",
        search_url,
        blob_names.len(),
        query
    );

    let payload = serde_json::json!({
        "information_request": query,
        "blobs": {"checkpoint_id": serde_json::Value::Null, "added_blobs": blob_names, "deleted_blobs": []},
        "dialog": [],
        "max_output_length": 0,
        "disable_codebase_retrieval": false,
        "enable_commit_retrieval": false,
    });

    // 创建 HTTP 客户端（支持代理）
    let client = create_acemcp_client(config)?;
    let value: serde_json::Value = retry_request(
        || async {
            let r = client
                .post(&search_url)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            let status = r.status();
            log_important!(info, "检索请求HTTP响应状态: {}", status);

            if status == StatusCode::UNAUTHORIZED {
                let body = r.text().await.unwrap_or_default();
                log_important!(info, "检索请求遇到 ACE 认证失败: {}", body);
                mark_project_auth_failure(project_root_path, Some(current_scope_hash.as_str()));
                anyhow::bail!(ACE_AUTH_FAILURE_SEARCH_MESSAGE);
            }

            if !status.is_success() {
                let body = r.text().await.unwrap_or_default();
                anyhow::bail!("HTTP {} {}", status, body);
            }

            let v: serde_json::Value = r.json().await?;
            // 只记录摘要，避免将 formatted_retrieval（可能包含大量代码片段）写入日志
            let keys: Vec<String> = v
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            let formatted_len = v
                .get("formatted_retrieval")
                .and_then(|x| x.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            log_important!(
                info,
                "检索响应摘要: keys={:?}, formatted_retrieval_len={}",
                keys,
                formatted_len
            );
            Ok(v)
        },
        3,
        2.0,
    )
    .await?;

    let text = value
        .get("formatted_retrieval")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if text.is_empty() {
        log_important!(info, "搜索返回空结果");
        Ok("No relevant code context found for your query.".to_string())
    } else {
        log_important!(info, "搜索成功，返回文本长度: {}", text.len());
        Ok(text)
    }
}

/// 创建支持代理的 HTTP 客户端
/// 根据配置决定是否使用代理
fn create_acemcp_client(config: &AcemcpConfig) -> anyhow::Result<Client> {
    let mut client_builder = Client::builder().timeout(Duration::from_secs(60));

    // 检查是否启用代理
    if config.proxy_enabled.unwrap_or(false) {
        let host = config
            .proxy_host
            .clone()
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = config.proxy_port.unwrap_or(7890);
        let proxy_type = config
            .proxy_type
            .clone()
            .unwrap_or_else(|| "http".to_string());

        // 校验代理类型，避免拼接出无效 URL
        match proxy_type.as_str() {
            "http" | "https" | "socks5" => {}
            other => anyhow::bail!("不支持的代理类型: {}（仅支持 http/https/socks5）", other),
        }

        // 仅用于日志提示（避免泄露密码）
        let has_auth = config
            .proxy_username
            .as_deref()
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);

        if has_auth {
            log_important!(
                info,
                "🔧 使用代理: {}://{}:{}（带认证）",
                proxy_type,
                host,
                port
            );
        } else {
            log_important!(info, "🔧 使用代理: {}://{}:{}", proxy_type, host, port);
        }

        // 构建代理 URL
        let proxy_url = format!("{}://{}:{}", proxy_type, host, port);

        // 使用 Proxy::all() 让所有请求都走代理
        let mut reqwest_proxy =
            reqwest::Proxy::all(&proxy_url).map_err(|e| anyhow::anyhow!("创建代理失败: {}", e))?;

        // 代理认证（Basic Auth）
        if let Some(username) = config.proxy_username.as_deref() {
            let username = username.trim();
            if !username.is_empty() {
                let password = config.proxy_password.as_deref().unwrap_or("");
                reqwest_proxy = reqwest_proxy.basic_auth(username, password);
            }
        }

        client_builder = client_builder.proxy(reqwest_proxy);
    } else {
        log_debug!("使用直连模式（未启用代理）");
    }

    client_builder
        .build()
        .map_err(|e| anyhow::anyhow!("构建 HTTP 客户端失败: {}", e))
}
