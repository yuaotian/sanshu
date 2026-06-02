use anyhow::{anyhow, Context, Result};
use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use crate::config::load_standalone_config;
use crate::log_important;
use crate::mcp::tools::acemcp::types::AcemcpRequest;
use crate::mcp::tools::AcemcpTool;

pub(crate) mod fast_context;

const BACKEND_ACE: &str = "ace";
const BACKEND_FAST_CONTEXT: &str = "fast_context";
const BACKEND_AUTO: &str = "auto";
const BACKEND_BOTH: &str = "both";
const BACKEND_DEFAULT: &str = "default";

/// sou 对外请求。旧客户端只传 project_root_path/query 时仍然可用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SouRequest {
    pub project_root_path: String,
    pub query: String,
    pub backend: Option<String>,
    pub tree_depth: Option<u8>,
    pub max_turns: Option<u8>,
    pub max_results: Option<u8>,
    pub max_commands: Option<u8>,
    pub timeout_ms: Option<u64>,
    pub exclude_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
struct SouRuntimeConfig {
    default_backend: String,
    auto_order: Vec<String>,
    include_backend_headers: bool,
    include_failed_backend_errors: bool,
    fast_context: FastContextConfig,
}

#[derive(Debug, Clone)]
struct FastContextConfig {
    api_key: Option<String>,
    tree_depth: u8,
    max_turns: u8,
    max_results: u8,
    max_commands: u8,
    timeout_ms: u64,
    exclude_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct BackendRunResult {
    backend: String,
    text: String,
}

#[derive(Debug, Clone)]
struct BackendRunError {
    backend: String,
    message: String,
}

pub struct SouTool;

impl SouTool {
    pub fn get_tool_definition() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "project_root_path": {
                    "type": "string",
                    "description": "项目根目录的绝对路径，使用正斜杠(/)作为分隔符。"
                },
                "query": {
                    "type": "string",
                    "description": "用于查找相关代码上下文的自然语言搜索查询。提示：代码标识符通常为英文，使用中文描述时建议混入英文类名/函数名/文件名（如 GestureRecognizer、ImageCodec），可以显著提升命中率与稳定性。"
                },
                "backend": {
                    "type": "string",
                    "enum": ["default", "auto", "ace", "fast_context", "both"],
                    "description": "可选搜索后端。default 使用配置；auto 按优先级自动回退；both 同时返回 ACE 与 fast-context。"
                },
                "tree_depth": {
                    "type": "number",
                    "description": "fast-context 目录树深度，范围 1-6。"
                },
                "max_turns": {
                    "type": "number",
                    "description": "fast-context 搜索轮数，范围 1-5。"
                },
                "max_results": {
                    "type": "number",
                    "description": "fast-context 最大返回文件数，范围 1-30。"
                },
                "max_commands": {
                    "type": "number",
                    "description": "fast-context 每轮最大本地命令数。"
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "fast-context 单次请求超时毫秒数。"
                },
                "exclude_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "fast-context 额外排除路径或 glob。"
                }
            },
            "required": ["project_root_path", "query"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            Tool {
                name: Cow::Borrowed("sou"),
                description: Some(Cow::Borrowed(
                    "代码上下文检索工具。支持 ACE、fast-context、自动回退与双后端合并返回。\n\n查询建议：\n- 代码标识符通常为英文，使用中文时建议混入英文类名/函数名/文件名（如 GestureRecognizer、ImageCodec、ClipboardService）。\n- 长中文描述容易让模型空 answer；如果第一次返回 0 结果，请拆成更具体的子问题或显式给出英文关键词重试。\n- 给出模块/目录提示（如 'gesture 模块' / 'src/capture/'）有助于快速定位。",
                )),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("代码搜索".to_string()),
            }
        } else {
            panic!("Schema creation failed");
        }
    }

    pub async fn search_context(request: SouRequest) -> Result<CallToolResult, McpError> {
        let config = SouRuntimeConfig::load()
            .map_err(|e| McpError::internal_error(format!("读取 sou 配置失败: {}", e), None))?;
        let strategy = resolve_strategy(request.backend.as_deref(), &config);

        log_important!(
            info,
            "[sou] 搜索请求: backend={}, project_root_path={}, query={}",
            strategy,
            request.project_root_path,
            request.query
        );

        match strategy.as_str() {
            BACKEND_ACE => {
                result_to_call_tool(run_ace(&request).await.map_err(|e| BackendRunError {
                    backend: BACKEND_ACE.to_string(),
                    message: e,
                }))
            }
            BACKEND_FAST_CONTEXT => result_to_call_tool(
                run_fast_context(
                    &request,
                    &config.fast_context,
                    config.include_backend_headers,
                )
                .await
                .map_err(|e| BackendRunError {
                    backend: BACKEND_FAST_CONTEXT.to_string(),
                    message: e,
                }),
            ),
            BACKEND_BOTH => run_both(&request, &config).await,
            BACKEND_AUTO => run_auto(&request, &config).await,
            other => Ok(error_result(format!("sou搜索失败: 未知后端策略 {}", other))),
        }
    }
}

impl SouRuntimeConfig {
    fn load() -> Result<Self> {
        let app_config =
            load_standalone_config().map_err(|e| anyhow!("读取配置文件失败: {}", e))?;
        let mcp = app_config.mcp_config;

        Ok(Self {
            default_backend: normalize_backend(
                mcp.sou_default_backend.as_deref().unwrap_or(BACKEND_AUTO),
            )
            .unwrap_or_else(|| BACKEND_AUTO.to_string()),
            auto_order: normalize_auto_order(mcp.sou_auto_order),
            include_backend_headers: mcp.sou_include_backend_headers.unwrap_or(true),
            include_failed_backend_errors: mcp.sou_include_failed_backend_errors.unwrap_or(true),
            fast_context: FastContextConfig {
                api_key: mcp.fast_context_api_key.and_then(|s| {
                    if s.trim().is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                }),
                tree_depth: clamp_u8(mcp.fast_context_tree_depth.unwrap_or(3), 1, 6),
                max_turns: clamp_u8(mcp.fast_context_max_turns.unwrap_or(3), 1, 5),
                max_results: clamp_u8(mcp.fast_context_max_results.unwrap_or(10), 1, 30),
                max_commands: clamp_u8(mcp.fast_context_max_commands.unwrap_or(8), 1, 20),
                timeout_ms: mcp
                    .fast_context_timeout_ms
                    .unwrap_or(30000)
                    .clamp(1000, 300000),
                exclude_paths: mcp
                    .fast_context_exclude_paths
                    .unwrap_or_else(default_fast_excludes),
            },
        })
    }
}

fn resolve_strategy(request_backend: Option<&str>, config: &SouRuntimeConfig) -> String {
    let requested = request_backend
        .and_then(normalize_backend)
        .unwrap_or_else(|| BACKEND_DEFAULT.to_string());

    if requested == BACKEND_DEFAULT {
        config.default_backend.clone()
    } else {
        requested
    }
}

fn normalize_backend(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "" | BACKEND_DEFAULT => Some(BACKEND_DEFAULT.to_string()),
        BACKEND_AUTO => Some(BACKEND_AUTO.to_string()),
        BACKEND_ACE | "acemcp" | "augment" => Some(BACKEND_ACE.to_string()),
        BACKEND_FAST_CONTEXT | "fastcontext" | "fast" => Some(BACKEND_FAST_CONTEXT.to_string()),
        BACKEND_BOTH | "all" | "merge" => Some(BACKEND_BOTH.to_string()),
        _ => None,
    }
}

fn normalize_auto_order(value: Option<Vec<String>>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for backend in
        value.unwrap_or_else(|| vec![BACKEND_ACE.to_string(), BACKEND_FAST_CONTEXT.to_string()])
    {
        if let Some(normalized) = normalize_backend(&backend) {
            if matches!(normalized.as_str(), BACKEND_ACE | BACKEND_FAST_CONTEXT)
                && seen.insert(normalized.clone())
            {
                out.push(normalized);
            }
        }
    }
    if out.is_empty() {
        out.push(BACKEND_ACE.to_string());
        out.push(BACKEND_FAST_CONTEXT.to_string());
    }
    out
}

async fn run_auto(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<CallToolResult, McpError> {
    let mut errors = Vec::new();
    for backend in &config.auto_order {
        log_important!(info, "[sou] auto 尝试后端: {}", backend);
        let result = match backend.as_str() {
            BACKEND_ACE => run_ace(request).await,
            BACKEND_FAST_CONTEXT => {
                run_fast_context(
                    request,
                    &config.fast_context,
                    config.include_backend_headers,
                )
                .await
            }
            _ => continue,
        };

        match result {
            Ok(ok) => {
                log_important!(info, "[sou] auto 后端成功: {}", ok.backend);
                return Ok(success_result(ok.text));
            }
            Err(message) => errors.push(BackendRunError {
                backend: backend.clone(),
                message,
            }),
        }
    }

    Ok(error_result(format_backend_errors(
        "sou搜索失败: 所有后端均不可用",
        &errors,
    )))
}

async fn run_both(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<CallToolResult, McpError> {
    let (ace, fast) = tokio::join!(
        run_ace(request),
        run_fast_context(
            request,
            &config.fast_context,
            config.include_backend_headers
        ),
    );

    let mut outputs = Vec::new();
    let mut errors = Vec::new();
    match ace {
        Ok(result) => {
            log_important!(info, "[sou] both 后端成功: ace");
            outputs.push(result);
        }
        Err(message) => {
            log_important!(warn, "[sou] both 后端失败: ace, error={}", message);
            errors.push(BackendRunError {
                backend: BACKEND_ACE.to_string(),
                message,
            });
        }
    }
    match fast {
        Ok(result) => {
            log_important!(info, "[sou] both 后端成功: fast_context");
            outputs.push(result);
        }
        Err(message) => {
            log_important!(warn, "[sou] both 后端失败: fast_context, error={}", message);
            errors.push(BackendRunError {
                backend: BACKEND_FAST_CONTEXT.to_string(),
                message,
            });
        }
    }

    if outputs.is_empty() {
        return Ok(error_result(format_backend_errors(
            "sou搜索失败: 所有后端均不可用",
            &errors,
        )));
    }

    let mut text = outputs
        .into_iter()
        .map(|result| {
            if config.include_backend_headers {
                format!("### sou backend: {}\n\n{}", result.backend, result.text)
            } else {
                result.text
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if config.include_failed_backend_errors && !errors.is_empty() {
        text.push_str("\n\n---\n后端诊断：\n");
        text.push_str(&format_backend_errors("", &errors));
    }

    Ok(success_result(text))
}

fn result_to_call_tool(
    result: Result<BackendRunResult, BackendRunError>,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(ok) => Ok(success_result(ok.text)),
        Err(err) => Ok(error_result(format!(
            "{}搜索失败: {}",
            backend_display(&err.backend),
            err.message
        ))),
    }
}

async fn run_ace(request: &SouRequest) -> Result<BackendRunResult, String> {
    let result = AcemcpTool::search_context(AcemcpRequest {
        project_root_path: request.project_root_path.clone(),
        query: request.query.clone(),
    })
    .await
    .map_err(|e| e.to_string())?;

    let text = call_result_text(&result);
    if result.is_error.unwrap_or(false) || is_ace_unavailable_text(&text) {
        return Err(text);
    }

    Ok(BackendRunResult {
        backend: BACKEND_ACE.to_string(),
        text,
    })
}

async fn run_fast_context(
    request: &SouRequest,
    config: &FastContextConfig,
    include_header: bool,
) -> Result<BackendRunResult, String> {
    let started_at = Instant::now();
    let project_root = canonical_project_root(&request.project_root_path)
        .map_err(|e| format!("项目路径无效: {}", e))?;
    let effective_timeout_ms = request
        .timeout_ms
        .unwrap_or(config.timeout_ms)
        .clamp(1000, 300000);
    let tree_depth = clamp_u8(request.tree_depth.unwrap_or(config.tree_depth), 1, 6);
    let max_turns = clamp_u8(request.max_turns.unwrap_or(config.max_turns), 1, 5);
    let max_results = clamp_u8(request.max_results.unwrap_or(config.max_results), 1, 30);
    let max_commands = clamp_u8(request.max_commands.unwrap_or(config.max_commands), 1, 20);
    let exclude_paths = request
        .exclude_paths
        .clone()
        .unwrap_or_else(|| config.exclude_paths.clone());

    log_important!(
        info,
        "[sou] fast-context 开始: project_root={}, query_len={}, timeout_ms={}, tree_depth={}, max_turns={}, max_results={}, max_commands={}, exclude_count={}, include_header={}",
        project_root,
        request.query.chars().count(),
        effective_timeout_ms,
        tree_depth,
        max_turns,
        max_results,
        max_commands,
        exclude_paths.len(),
        include_header
    );

    let response = tokio::time::timeout(
        Duration::from_millis(effective_timeout_ms + 5000),
        fast_context::search(fast_context::SearchOptions {
            query: request.query.clone(),
            project_root: PathBuf::from(&project_root),
            api_key: config.api_key.clone(),
            tree_depth,
            max_turns,
            max_results,
            max_commands,
            timeout_ms: effective_timeout_ms,
            exclude_paths,
        }),
    )
    .await
    .map_err(|_| {
        log_important!(
            warn,
            "[sou] fast-context 超时: timeout_ms={}, elapsed_ms={}",
            effective_timeout_ms,
            started_at.elapsed().as_millis()
        );
        format!("fast-context 超时（{}ms）", effective_timeout_ms)
    })?
    .map_err(|e| {
        let message = e.to_string();
        log_important!(
            warn,
            "[sou] fast-context 失败: elapsed_ms={}, error={}",
            started_at.elapsed().as_millis(),
            message
        );
        message
    })?;

    log_important!(
        info,
        "[sou] fast-context 原生结果: files={}, rg_patterns={}, meta={}",
        response.files.len(),
        response.rg_patterns.len(),
        response.meta
    );

    let text = format_fast_context_text(&project_root, &response, include_header).map_err(|e| {
        let message = e.to_string();
        log_important!(warn, "[sou] fast-context 格式化失败: {}", message);
        message
    })?;
    if text.trim().is_empty() {
        return Err("fast-context 未返回可用文件范围".to_string());
    }

    log_important!(
        info,
        "[sou] fast-context 完成: elapsed_ms={}, output_len={}",
        started_at.elapsed().as_millis(),
        text.len()
    );

    Ok(BackendRunResult {
        backend: BACKEND_FAST_CONTEXT.to_string(),
        text,
    })
}

fn format_fast_context_text(
    project_root: &str,
    response: &fast_context::SearchResult,
    include_header: bool,
) -> Result<String> {
    let root = PathBuf::from(project_root);
    let mut parts = Vec::new();
    if include_header {
        parts.push("The following code sections were retrieved:".to_string());
        parts.push(String::new());
    }

    log_important!(
        info,
        "[sou] fast-context 兼容格式化: files={}, include_header={}",
        response.files.len(),
        include_header
    );

    for file in &response.files {
        let Some(path) = resolve_fast_context_file(&root, file)? else {
            log_important!(warn, "[sou] fast-context 文件项缺少路径，已跳过");
            continue;
        };
        if !path.exists() || !path.is_file() {
            log_important!(
                warn,
                "[sou] fast-context 文件不存在或不是文件，已跳过: {}",
                path.display()
            );
            continue;
        }

        let display = normalize_path(&path);
        let ranges = if file.ranges.is_empty() {
            vec![[1, 80]]
        } else {
            file.ranges.clone()
        };

        for range in ranges {
            let start = range[0].max(1);
            let end = range[1].max(start).min(start.saturating_add(220));
            // #3 优先用 ToolExecutor 中已读取的文件内容（fast-context 阶段 readfile 命中）
            let cache_key = normalize_path(&path);
            let snippet = if let Some(content) = response.file_cache.get(&cache_key) {
                extract_line_range(content, start, end)
            } else {
                read_line_range(&path, start, end)?
            };
            if snippet.trim().is_empty() {
                log_important!(
                    warn,
                    "[sou] fast-context 片段为空，已跳过: path={}, range=L{}-L{}",
                    path.display(),
                    start,
                    end
                );
                continue;
            }
            log_important!(
                info,
                "[sou] fast-context 片段已格式化: path={}, range=L{}-L{}, snippet_len={}",
                path.display(),
                start,
                end,
                snippet.len()
            );
            parts.push(format!("Path: {}", display));
            parts.push(format!("Lines: L{}-L{}", start, end));
            parts.push(snippet);
            parts.push(String::new());
        }
    }

    if !response.rg_patterns.is_empty() {
        parts.push(format!(
            "grep keywords: {}",
            response.rg_patterns.join(", ")
        ));
    }
    parts.push(format!(
        "[fast-context stats] commands_seen={}, commands_executed={}, commands_useful={}, commands_invalid={}, repaired={}, path_missing={}, path_repaired={}, cache_hits={}, useful_command_rate={}%, invalid_command_rate={}%",
        response.stats.commands_seen,
        response.stats.commands_executed,
        response.stats.commands_useful,
        response.stats.commands_invalid,
        response.stats.commands_repaired,
        response.stats.path_missing,
        response.stats.path_repaired,
        response.stats.cache_hits,
        response.stats.useful_rate(),
        response.stats.invalid_rate()
    ));
    if parts
        .iter()
        .all(|line| line.trim().is_empty() || line.starts_with("The following"))
    {
        parts.push("No relevant files found.".to_string());
    }
    if !response.meta.is_null() {
        parts.push(format!("[fast-context config] {}", response.meta));
    }

    Ok(parts.join("\n"))
}

fn resolve_fast_context_file(root: &Path, file: &FastContextFile) -> Result<Option<PathBuf>> {
    let candidate = if let Some(full_path) = file.full_path.as_deref() {
        PathBuf::from(full_path)
    } else if let Some(path) = file.path.as_deref() {
        root.join(path)
    } else {
        return Ok(None);
    };
    let absolute = candidate.canonicalize().unwrap_or(candidate);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !absolute.starts_with(&root) {
        return Err(anyhow!(
            "fast-context 返回了项目外路径: {}",
            absolute.display()
        ));
    }
    Ok(Some(absolute))
}

fn read_line_range(path: &Path, start: usize, end: usize) -> Result<String> {
    let content =
        fs::read_to_string(path).with_context(|| format!("读取文件失败: {}", path.display()))?;
    Ok(extract_line_range(&content, start, end))
}

/// 从已知文件内容中切片指定行范围；与 read_line_range 输出格式保持一致
fn extract_line_range(content: &str, start: usize, end: usize) -> String {
    let mut out = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let line_no = index + 1;
        if line_no >= start && line_no <= end {
            out.push(format!("L{}:{}", line_no, line));
        }
        if line_no > end {
            break;
        }
    }
    out.join("\n")
}

fn call_result_text(result: &CallToolResult) -> String {
    let value = serde_json::to_value(&result.content).unwrap_or_default();
    value
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|first| {
            first
                .get("text")
                .and_then(|v| v.as_str())
                .or_else(|| first.get("data").and_then(|v| v.as_str()))
        })
        .unwrap_or_default()
        .to_string()
}

fn is_ace_unavailable_text(text: &str) -> bool {
    let normalized = text.trim();
    normalized.is_empty()
        || normalized.starts_with("Acemcp搜索失败:")
        || normalized.starts_with("搜索失败:")
        || normalized.starts_with("索引更新失败:")
        || normalized.starts_with("代码搜索失败:")
        || normalized.contains("未配置 base_url")
        || normalized.contains("未配置 token")
        || normalized.contains("认证失败")
        || normalized.contains("尚未建立索引")
        || normalized.contains("正在后台索引")
        || normalized.contains("索引尚未就绪")
        || normalized.contains("配置已变更")
}

fn success_result(text: String) -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(text)],
        is_error: None,
        meta: None,
        structured_content: None,
    }
}

fn error_result(text: String) -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(text)],
        is_error: Some(true),
        meta: None,
        structured_content: None,
    }
}

fn format_backend_errors(prefix: &str, errors: &[BackendRunError]) -> String {
    let mut lines = Vec::new();
    if !prefix.is_empty() {
        lines.push(prefix.to_string());
    }
    for err in errors {
        lines.push(format!(
            "- {}: {}",
            backend_display(&err.backend),
            err.message
        ));
    }
    lines.join("\n")
}

fn backend_display(backend: &str) -> &'static str {
    match backend {
        BACKEND_ACE => "ACE",
        BACKEND_FAST_CONTEXT => "FastContext",
        _ => "sou",
    }
}

fn canonical_project_root(path: &str) -> Result<String> {
    let root = PathBuf::from(path);
    let canonical = root
        .canonicalize()
        .with_context(|| format!("无法解析项目路径: {}", path))?;
    if !canonical.is_dir() {
        return Err(anyhow!("项目路径不是目录: {}", canonical.display()));
    }
    Ok(normalize_path(&canonical))
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn clamp_u8(value: u8, min: u8, max: u8) -> u8 {
    value.max(min).min(max)
}

fn default_fast_excludes() -> Vec<String> {
    vec![
        "node_modules".to_string(),
        ".git".to_string(),
        "dist".to_string(),
        "build".to_string(),
        "target".to_string(),
        "coverage".to_string(),
    ]
}

type FastContextFile = fast_context::FastContextFile;
