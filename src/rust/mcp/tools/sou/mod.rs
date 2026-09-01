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
pub(crate) mod local;
pub(crate) mod reranker;
pub(crate) mod semantic;

const BACKEND_ACE: &str = "ace";
const BACKEND_FAST_CONTEXT: &str = "fast_context";
const BACKEND_LOCAL: &str = "local";
const BACKEND_AUTO: &str = "auto";
const BACKEND_BOTH: &str = "both";
const BACKEND_DEFAULT: &str = "default";
const FAST_CONTEXT_FALLBACK_RETRY_DELAY_MS: u64 = 700;

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

/// crate 内部统一代码片段，供 uiux 等组合工具消费，避免重复解析 MCP 文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SouSection {
    pub backend: String,
    pub location: String,
    pub excerpt: String,
}

#[derive(Debug, Clone)]
struct SouRuntimeConfig {
    default_backend: String,
    auto_order: Vec<String>,
    include_backend_headers: bool,
    include_failed_backend_errors: bool,
    local_enabled: bool,
    local_index_dir: PathBuf,
    local_semantic: local::LocalSemanticSettings,
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
    hit_count: usize,
    duration_ms: u64,
    engine: Option<String>,
    index_state: Option<String>,
    fallback_reason: Option<String>,
    semantic_state: Option<String>,
    semantic_model: Option<String>,
    semantic_indexed_chunks: Option<u64>,
    semantic_pending_chunks: Option<u64>,
    semantic_top_score: Option<f32>,
    semantic_mode: Option<String>,
    reranker_state: Option<String>,
    reranker_model: Option<String>,
    reranker_duration_ms: Option<u64>,
    reranker_top_score: Option<f32>,
    fusion: Option<String>,
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
                    "enum": ["default", "auto", "ace", "fast_context", "local", "both"],
                    "description": "可选搜索后端。default 使用配置；auto 按优先级自动回退；local 使用本地 FTS5/rg；both 同时返回 ACE 与 fast-context。"
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
                    "description": "auto 模式远端后端总预算；显式 fast-context 模式下为单次请求超时毫秒数。"
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
                    "代码上下文检索工具。支持 ACE、fast-context、本地 FTS5/rg 兜底、自动回退与双后端合并返回。\n\n查询建议：\n- 代码标识符通常为英文，使用中文时建议混入英文类名/函数名/文件名（如 GestureRecognizer、ImageCodec、ClipboardService）。\n- 长中文描述容易让模型空 answer；如果第一次返回 0 结果，请拆成更具体的子问题或显式给出英文关键词重试。\n- 给出模块/目录提示（如 'gesture 模块' / 'src/capture/'）有助于快速定位。",
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
            "[sou] 搜索请求: backend={}, query_chars={}",
            strategy,
            request.query.chars().count()
        );

        match strategy.as_str() {
            BACKEND_ACE => result_to_call_tool(
                run_ace(&request).await.map_err(|e| BackendRunError {
                    backend: BACKEND_ACE.to_string(),
                    message: e,
                }),
                BACKEND_ACE,
            ),
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
                BACKEND_FAST_CONTEXT,
            ),
            BACKEND_LOCAL if config.local_enabled => result_to_call_tool(
                run_local(&request, &config)
                    .await
                    .map_err(|e| BackendRunError {
                        backend: BACKEND_LOCAL.to_string(),
                        message: e,
                    }),
                BACKEND_LOCAL,
            ),
            BACKEND_LOCAL => Ok(error_result("Local搜索失败: 本地兜底已禁用".to_string())),
            BACKEND_BOTH => run_both(&request, &config).await,
            BACKEND_AUTO => run_auto(&request, &config).await,
            other => Ok(error_result(format!("sou搜索失败: 未知后端策略 {}", other))),
        }
    }

    /// 内部结构化搜索入口；对外 MCP 文本协议继续由 search_context 保持兼容。
    pub(crate) async fn search_sections(request: SouRequest) -> Result<Vec<SouSection>, String> {
        let config =
            SouRuntimeConfig::load().map_err(|error| format!("读取 sou 配置失败: {}", error))?;
        let strategy = resolve_strategy(request.backend.as_deref(), &config);
        let results = match strategy.as_str() {
            BACKEND_ACE => vec![run_ace(&request).await?],
            BACKEND_FAST_CONTEXT => vec![
                run_fast_context(
                    &request,
                    &config.fast_context,
                    config.include_backend_headers,
                )
                .await?,
            ],
            BACKEND_LOCAL if config.local_enabled => {
                vec![run_local(&request, &config).await?]
            }
            BACKEND_LOCAL => return Err("Local搜索失败: 本地兜底已禁用".to_string()),
            BACKEND_AUTO => vec![run_auto_result(&request, &config).await.map_err(|errors| {
                format_backend_errors("sou搜索失败: 所有后端均不可用", &errors)
            })?],
            BACKEND_BOTH => {
                let (results, errors) = run_both_results(&request, &config).await;
                if results.is_empty() {
                    return Err(format_backend_errors(
                        "sou搜索失败: 所有后端均不可用",
                        &errors,
                    ));
                }
                results
            }
            other => return Err(format!("sou搜索失败: 未知后端策略 {}", other)),
        };
        let sections = results
            .into_iter()
            .flat_map(|result| parse_sou_sections(&result.text, &result.backend))
            .collect::<Vec<_>>();
        if sections.is_empty() {
            Err("sou 未返回可解析的代码片段".to_string())
        } else {
            Ok(sections)
        }
    }
}

impl SouRuntimeConfig {
    fn load() -> Result<Self> {
        let app_config =
            load_standalone_config().map_err(|e| anyhow!("读取配置文件失败: {}", e))?;
        let mcp = app_config.mcp_config;

        let semantic_mode = crate::config::effective_sou_semantic_mode(
            mcp.sou_local_semantic_mode.as_deref(),
            mcp.sou_local_semantic_enabled,
        );
        let local_semantic = local::LocalSemanticSettings {
            mode: local::LocalSemanticMode::from_effective(semantic_mode),
            model_dir: crate::mcp::embedding::effective_model_dir(
                mcp.local_embedding_model_dir.as_deref(),
                mcp.uiux_model_dir.as_deref(),
            ),
            reranker_model_dir: crate::config::effective_sou_reranker_model_dir(
                mcp.sou_reranker_model_dir.as_deref(),
            ),
        };
        Ok(Self {
            default_backend: normalize_backend(
                mcp.sou_default_backend.as_deref().unwrap_or(BACKEND_AUTO),
            )
            .unwrap_or_else(|| BACKEND_AUTO.to_string()),
            auto_order: normalize_auto_order(mcp.sou_auto_order),
            include_backend_headers: mcp.sou_include_backend_headers.unwrap_or(true),
            include_failed_backend_errors: mcp.sou_include_failed_backend_errors.unwrap_or(true),
            local_enabled: mcp.sou_local_enabled.unwrap_or(true),
            local_index_dir: crate::config::effective_sou_local_index_dir(
                mcp.sou_local_index_dir.as_deref(),
            ),
            local_semantic,
            fast_context: FastContextConfig {
                api_key: mcp.fast_context_api_key.and_then(|s| {
                    if s.trim().is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                }),
                tree_depth: clamp_u8(mcp.fast_context_tree_depth.unwrap_or(3), 1, 6),
                max_turns: clamp_u8(mcp.fast_context_max_turns.unwrap_or(4), 1, 5),
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

/// 当前 sou 后端策略是否包含 fast-context（default/both 直达，或 auto 顺序中包含）。
/// 供 uiux 等上层工具判断"用户是否开启了 fast-context 检索链路"。
pub fn fast_context_in_strategy() -> bool {
    let Ok(config) = SouRuntimeConfig::load() else {
        return false;
    };
    match config.default_backend.as_str() {
        BACKEND_FAST_CONTEXT | BACKEND_BOTH => true,
        BACKEND_AUTO => config
            .auto_order
            .iter()
            .any(|backend| backend == BACKEND_FAST_CONTEXT),
        _ => false,
    }
}

/// 是否能在本地检测到 fast-context API Key（配置 → 环境变量 → Devin/Windsurf 登录库）。
/// 此函数不发起远端请求；Key 的实际有效性由后续检索结果确认。
pub fn fast_context_key_detected() -> bool {
    let Ok(config) = SouRuntimeConfig::load() else {
        return false;
    };
    fast_context::detect_api_key(config.fast_context.api_key.as_deref()).is_ok()
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
        BACKEND_LOCAL | "offline" | "rg" => Some(BACKEND_LOCAL.to_string()),
        BACKEND_BOTH | "all" | "merge" => Some(BACKEND_BOTH.to_string()),
        _ => None,
    }
}

fn normalize_auto_order(value: Option<Vec<String>>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for backend in value.unwrap_or_else(|| {
        vec![
            BACKEND_ACE.to_string(),
            BACKEND_FAST_CONTEXT.to_string(),
            BACKEND_LOCAL.to_string(),
        ]
    }) {
        if let Some(normalized) = normalize_backend(&backend) {
            if matches!(
                normalized.as_str(),
                BACKEND_ACE | BACKEND_FAST_CONTEXT | BACKEND_LOCAL
            ) && seen.insert(normalized.clone())
            {
                out.push(normalized);
            }
        }
    }
    if out.is_empty() {
        out.push(BACKEND_ACE.to_string());
        out.push(BACKEND_FAST_CONTEXT.to_string());
    }
    if seen.insert(BACKEND_LOCAL.to_string()) {
        out.push(BACKEND_LOCAL.to_string());
    }
    out
}

async fn run_auto(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<CallToolResult, McpError> {
    match run_auto_result(request, config).await {
        Ok(result) => Ok(backend_success_result(
            result,
            BACKEND_AUTO,
            config.include_failed_backend_errors,
        )),
        Err(errors) => Ok(error_result(format_backend_errors(
            "sou搜索失败: 所有后端均不可用",
            &errors,
        ))),
    }
}

async fn run_auto_result(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<BackendRunResult, Vec<BackendRunError>> {
    let mut errors = Vec::new();
    let remote_backend_count = config
        .auto_order
        .iter()
        .filter(|backend| matches!(backend.as_str(), BACKEND_ACE | BACKEND_FAST_CONTEXT))
        .count()
        .max(1) as u64;
    let total_remote_budget_ms = request
        .timeout_ms
        .unwrap_or(config.fast_context.timeout_ms)
        .clamp(1000, 300000);
    let per_remote_timeout_ms = (total_remote_budget_ms / remote_backend_count).max(1000);

    for backend in &config.auto_order {
        log_important!(info, "[sou] auto 尝试后端: {}", backend);
        let result = match backend.as_str() {
            BACKEND_ACE => match tokio::time::timeout(
                Duration::from_millis(per_remote_timeout_ms),
                run_ace(request),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(format!(
                    "ACE 自动回退预算超时（{}ms），继续尝试下一后端",
                    per_remote_timeout_ms
                )),
            },
            BACKEND_FAST_CONTEXT => {
                match tokio::time::timeout(
                    Duration::from_millis(per_remote_timeout_ms),
                    run_fast_context(
                        request,
                        &config.fast_context,
                        config.include_backend_headers,
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(format!(
                        "FastContext 自动回退预算超时（{}ms），继续尝试下一后端",
                        per_remote_timeout_ms
                    )),
                }
            }
            BACKEND_LOCAL if config.local_enabled => run_local(request, config).await,
            BACKEND_LOCAL => Err("本地兜底已禁用".to_string()),
            _ => continue,
        };

        match result {
            Ok(ok) if ok.hit_count == 0 && backend != BACKEND_LOCAL => {
                log_important!(info, "[sou] auto 后端无命中，继续回退: {}", ok.backend);
                errors.push(BackendRunError {
                    backend: backend.clone(),
                    message: "返回 0 个代码片段".to_string(),
                });
            }
            Ok(mut ok) => {
                if !errors.is_empty() {
                    let prior = format_backend_errors("", &errors);
                    ok.fallback_reason = Some(match ok.fallback_reason.take() {
                        Some(current) => format!("{}；{}", prior, current),
                        None => prior,
                    });
                }
                log_important!(info, "[sou] auto 后端成功: {}", ok.backend);
                return Ok(ok);
            }
            Err(message) => errors.push(BackendRunError {
                backend: backend.clone(),
                message,
            }),
        }
    }

    Err(errors)
}

async fn run_both(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<CallToolResult, McpError> {
    let (outputs, errors) = run_both_results(request, config).await;

    if outputs.is_empty() {
        return Ok(error_result(format_backend_errors(
            "sou搜索失败: 所有后端均不可用",
            &errors,
        )));
    }

    let mut text = outputs
        .iter()
        .map(|result| {
            if config.include_backend_headers {
                format!("### sou backend: {}\n\n{}", result.backend, result.text)
            } else {
                result.text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if config.include_failed_backend_errors && !errors.is_empty() {
        text.push_str("\n\n---\n后端诊断：\n");
        text.push_str(&format_backend_errors("", &errors));
    }

    let total_hits = outputs.iter().map(|result| result.hit_count).sum::<usize>();
    let total_duration_ms = outputs
        .iter()
        .map(|result| result.duration_ms)
        .max()
        .unwrap_or_default();
    Ok(success_result_with_metadata(
        text,
        serde_json::json!({
            "requested_backend": BACKEND_BOTH,
            "actual_backend": BACKEND_BOTH,
            "degraded": !errors.is_empty(),
            "hit_count": total_hits,
            "duration_ms": total_duration_ms,
            "fallback_reason": if errors.is_empty() { None } else { Some(format_backend_errors("", &errors)) },
        }),
    ))
}

async fn run_both_results(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> (Vec<BackendRunResult>, Vec<BackendRunError>) {
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

    (outputs, errors)
}

fn result_to_call_tool(
    result: Result<BackendRunResult, BackendRunError>,
    requested_backend: &str,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(ok) => Ok(backend_success_result(ok, requested_backend, true)),
        Err(err) => Ok(error_result(format!(
            "{}搜索失败: {}",
            backend_display(&err.backend),
            err.message
        ))),
    }
}

async fn run_ace(request: &SouRequest) -> Result<BackendRunResult, String> {
    let started_at = Instant::now();
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
        hit_count: parse_sou_sections(&text, BACKEND_ACE).len(),
        duration_ms: started_at.elapsed().as_millis() as u64,
        engine: None,
        index_state: None,
        fallback_reason: None,
        semantic_state: None,
        semantic_model: None,
        semantic_indexed_chunks: None,
        semantic_pending_chunks: None,
        semantic_top_score: None,
        semantic_mode: None,
        reranker_state: None,
        reranker_model: None,
        reranker_duration_ms: None,
        reranker_top_score: None,
        fusion: None,
        text,
    })
}

async fn run_local(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<BackendRunResult, String> {
    let defaults = &config.fast_context;
    let output = local::search(local::LocalSearchOptions {
        project_root: PathBuf::from(&request.project_root_path),
        query: request.query.clone(),
        max_results: request.max_results.unwrap_or(defaults.max_results) as usize,
        exclude_paths: request
            .exclude_paths
            .clone()
            .unwrap_or_else(|| defaults.exclude_paths.clone()),
        index_dir: config.local_index_dir.clone(),
        semantic: config.local_semantic.clone(),
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(BackendRunResult {
        backend: BACKEND_LOCAL.to_string(),
        text: output.text,
        hit_count: output.hit_count,
        duration_ms: output.duration_ms,
        engine: Some(output.engine),
        index_state: Some(output.index_state),
        fallback_reason: output.fallback_reason,
        semantic_state: Some(output.semantic_state),
        semantic_model: output.semantic_model,
        semantic_indexed_chunks: Some(output.semantic_indexed_chunks),
        semantic_pending_chunks: Some(output.semantic_pending_chunks),
        semantic_top_score: output.semantic_top_score,
        semantic_mode: Some(output.semantic_mode),
        reranker_state: output.reranker_state,
        reranker_model: output.reranker_model,
        reranker_duration_ms: output.reranker_duration_ms,
        reranker_top_score: output.reranker_top_score,
        fusion: output.fusion,
    })
}

async fn run_fast_context(
    request: &SouRequest,
    config: &FastContextConfig,
    include_header: bool,
) -> Result<BackendRunResult, String> {
    let first = run_fast_context_once(request, config, include_header, false).await;
    match first {
        Ok(result) => Ok(result),
        Err(message) if should_retry_fast_context_search(&message) => {
            log_important!(
                warn,
                "[sou] fast-context 触发独立兜底重试: delay_ms={}, first_error={}",
                FAST_CONTEXT_FALLBACK_RETRY_DELAY_MS,
                message
            );
            // 兜底重试只在退化场景发生，短延迟用于避免连续完整会话给远端服务造成瞬时压力。
            tokio::time::sleep(Duration::from_millis(FAST_CONTEXT_FALLBACK_RETRY_DELAY_MS)).await;
            run_fast_context_once(request, config, include_header, true)
                .await
                .map_err(|retry_error| {
                    format!("{}；兜底重试仍失败: {}", message.trim(), retry_error.trim())
                })
        }
        Err(message) => Err(message),
    }
}

async fn run_fast_context_once(
    request: &SouRequest,
    config: &FastContextConfig,
    include_header: bool,
    fallback_attempt: bool,
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
        "[sou] fast-context 开始: fallback_attempt={}, project_root={}, query_len={}, timeout_ms={}, tree_depth={}, max_turns={}, max_results={}, max_commands={}, exclude_count={}, include_header={}",
        fallback_attempt,
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
        "[sou] fast-context 原生结果: fallback_attempt={}, files={}, answer_received={}, rg_patterns={}, meta={}",
        fallback_attempt,
        response.files.len(),
        response.answer_received,
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
    if !response.answer_received {
        return Err("fast-context 未获得合法 answer".to_string());
    }

    log_important!(
        info,
        "[sou] fast-context 完成: fallback_attempt={}, elapsed_ms={}, output_len={}",
        fallback_attempt,
        started_at.elapsed().as_millis(),
        text.len()
    );

    Ok(BackendRunResult {
        backend: BACKEND_FAST_CONTEXT.to_string(),
        hit_count: parse_sou_sections(&text, BACKEND_FAST_CONTEXT).len(),
        duration_ms: started_at.elapsed().as_millis() as u64,
        engine: None,
        index_state: None,
        fallback_reason: None,
        semantic_state: None,
        semantic_model: None,
        semantic_indexed_chunks: None,
        semantic_pending_chunks: None,
        semantic_top_score: None,
        semantic_mode: None,
        reranker_state: None,
        reranker_model: None,
        reranker_duration_ms: None,
        reranker_top_score: None,
        fusion: None,
        text,
    })
}

fn should_retry_fast_context_search(message: &str) -> bool {
    message.contains("未获得合法工具调用")
        || message.contains("未知工具调用")
        || message.contains("未获得合法 answer")
        || message.contains("已达到最大轮次")
        || message.contains("未返回可解析响应")
}

fn format_fast_context_text(
    project_root: &str,
    response: &fast_context::SearchResult,
    include_header: bool,
) -> Result<String> {
    let root = PathBuf::from(project_root);
    let mut parts = Vec::new();
    let mut code_sections = 0usize;
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
            code_sections += 1;
        }
    }

    if code_sections == 0 && response.answer_received {
        parts.push("No relevant files found.".to_string());
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

fn parse_sou_sections(text: &str, default_backend: &str) -> Vec<SouSection> {
    let mut sections = Vec::new();
    let mut backend = default_backend.to_string();
    let mut current_location: Option<String> = None;
    let mut current_lines = Vec::new();
    let mut current_ace_markdown = false;
    let mut in_fenced_code = false;
    let lines = text.lines().collect::<Vec<_>>();

    for (index, line) in lines.iter().enumerate() {
        if in_fenced_code {
            if is_markdown_fence(line) {
                in_fenced_code = false;
            } else if current_location.is_some() {
                current_lines.push(line.trim_end().to_string());
            }
            continue;
        }

        if current_location.is_some() && is_markdown_fence(line) {
            in_fenced_code = true;
            continue;
        }

        if let Some(value) = line.strip_prefix("### sou backend: ") {
            flush_sou_section(
                &mut sections,
                &backend,
                &mut current_location,
                &mut current_lines,
            );
            backend = value.trim().to_string();
            current_ace_markdown = false;
            continue;
        }
        if let Some(path) = line.strip_prefix("Path: ") {
            flush_sou_section(
                &mut sections,
                &backend,
                &mut current_location,
                &mut current_lines,
            );
            current_location = Some(normalize_sou_location(path));
            current_ace_markdown = false;
            continue;
        }

        // ACE 当前响应使用“## 文件路径 + Lines: x-y + fenced code”格式。
        if backend == BACKEND_ACE {
            if let Some(path) = ace_markdown_path(line, &lines[index + 1..]) {
                flush_sou_section(
                    &mut sections,
                    &backend,
                    &mut current_location,
                    &mut current_lines,
                );
                current_location = Some(normalize_sou_location(path));
                current_ace_markdown = true;
                continue;
            }
        }

        if let Some(range) = normalize_sou_line_range(line) {
            if let Some(location) = current_location.as_mut() {
                *location = format!("{}:{}", location, range);
            }
            continue;
        }
        if current_ace_markdown && (line.starts_with("Score: ") || line.starts_with("Confidence: "))
        {
            continue;
        }
        if line.starts_with("The following code sections were retrieved:") {
            continue;
        }
        if line.starts_with("[sou metadata]")
            || line.starts_with("[sou fallback]")
            || line.starts_with("[sou-local]")
            || line.starts_with("[sou-local fallback]")
            || line.starts_with("[fast-context stats]")
            || line.starts_with("[fast-context config]")
            || line.starts_with("grep keywords:")
        {
            continue;
        }
        if current_location.is_some() {
            current_lines.push(line.trim_end().to_string());
        }
    }

    flush_sou_section(
        &mut sections,
        &backend,
        &mut current_location,
        &mut current_lines,
    );
    sections
}

fn is_markdown_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

fn ace_markdown_path<'a>(line: &'a str, following_lines: &[&str]) -> Option<&'a str> {
    let path = line.strip_prefix("## ")?.trim();
    if path.is_empty()
        || !following_lines
            .iter()
            .take(5)
            .any(|candidate| normalize_sou_line_range(candidate).is_some())
    {
        return None;
    }
    Some(path)
}

fn normalize_sou_line_range(line: &str) -> Option<String> {
    let raw = line.strip_prefix("Lines: ")?.trim();
    let normalized = raw.strip_prefix('L').unwrap_or(raw).replace("-L", "-");
    let (start, end) = normalized
        .split_once('-')
        .map(|(start, end)| (start, Some(end)))
        .unwrap_or((normalized.as_str(), None));
    let start_number = start.parse::<usize>().ok()?;
    let end_number = end.map(str::parse::<usize>).transpose().ok()??;
    if start_number == 0 || end_number < start_number {
        return None;
    }
    Some(format!("{}-{}", start_number, end_number))
}

fn normalize_sou_location(path: &str) -> String {
    let trimmed = path.trim();
    if let Some((location, range)) = trimmed.rsplit_once(" (L") {
        return format!(
            "{}:{}",
            location.trim(),
            range.trim_end_matches(')').replace("-L", "-")
        );
    }
    trimmed.to_string()
}

fn flush_sou_section(
    sections: &mut Vec<SouSection>,
    backend: &str,
    current_location: &mut Option<String>,
    current_lines: &mut Vec<String>,
) {
    let Some(location) = current_location.take() else {
        current_lines.clear();
        return;
    };
    let excerpt = current_lines
        .iter()
        .filter(|line| !line.trim().is_empty() && line.trim() != "...")
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    current_lines.clear();
    if excerpt.trim().is_empty() {
        return;
    }
    sections.push(SouSection {
        backend: backend.to_string(),
        location,
        excerpt,
    });
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

fn backend_success_result(
    result: BackendRunResult,
    requested_backend: &str,
    include_fallback_text: bool,
) -> CallToolResult {
    let degraded = result.fallback_reason.is_some();
    let mut text = result.text.clone();
    let diagnostics = [
        result
            .engine
            .as_deref()
            .map(|value| format!(", engine={}", value)),
        result
            .index_state
            .as_deref()
            .map(|value| format!(", index_state={}", value)),
        result
            .semantic_mode
            .as_deref()
            .map(|value| format!(", semantic_mode={}", value)),
        result
            .semantic_state
            .as_deref()
            .map(|value| format!(", semantic_state={}", value)),
        result
            .semantic_model
            .as_deref()
            .map(|value| format!(", semantic_model={}", value)),
        result
            .semantic_indexed_chunks
            .map(|value| format!(", semantic_indexed_chunks={}", value)),
        result
            .semantic_pending_chunks
            .map(|value| format!(", semantic_pending_chunks={}", value)),
        result
            .semantic_top_score
            .map(|value| format!(", semantic_top_score={:.4}", value)),
        result
            .reranker_state
            .as_deref()
            .map(|value| format!(", reranker_state={}", value)),
        result
            .reranker_model
            .as_deref()
            .map(|value| format!(", reranker_model={}", value)),
        result
            .reranker_duration_ms
            .map(|value| format!(", reranker_duration_ms={}", value)),
        result
            .reranker_top_score
            .map(|value| format!(", reranker_top_score={:.4}", value)),
        result
            .fusion
            .as_deref()
            .map(|value| format!(", fusion={}", value)),
    ]
    .into_iter()
    .flatten()
    .collect::<String>();
    text.push_str(&format!(
        "\n[sou metadata] requested_backend={}, actual_backend={}, degraded={}, hit_count={}, duration_ms={}{}",
        requested_backend,
        result.backend,
        degraded,
        result.hit_count,
        result.duration_ms,
        diagnostics,
    ));
    if include_fallback_text {
        if let Some(reason) = result.fallback_reason.as_deref() {
            text.push_str(&format!("\n[sou fallback] {}", diagnostic_summary(reason)));
        }
    }
    success_result_with_metadata(
        text,
        serde_json::json!({
            "requested_backend": requested_backend,
            "actual_backend": result.backend,
            "degraded": degraded,
            "hit_count": result.hit_count,
            "duration_ms": result.duration_ms,
            "engine": result.engine,
            "index_state": result.index_state,
            "fallback_reason": result.fallback_reason,
            "semantic_state": result.semantic_state,
            "semantic_model": result.semantic_model,
            "semantic_indexed_chunks": result.semantic_indexed_chunks,
            "semantic_pending_chunks": result.semantic_pending_chunks,
            "semantic_top_score": result.semantic_top_score,
            "semantic_mode": result.semantic_mode,
            "reranker_state": result.reranker_state,
            "reranker_model": result.reranker_model,
            "reranker_duration_ms": result.reranker_duration_ms,
            "reranker_top_score": result.reranker_top_score,
            "fusion": result.fusion,
        }),
    )
}

fn success_result_with_metadata(text: String, metadata: serde_json::Value) -> CallToolResult {
    CallToolResult {
        content: vec![Content::text(text)],
        is_error: Some(false),
        meta: None,
        structured_content: Some(metadata),
    }
}

fn diagnostic_summary(message: &str) -> String {
    let single_line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= 320 {
        return single_line;
    }
    format!("{}...", single_line.chars().take(320).collect::<String>())
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
        BACKEND_LOCAL => "Local",
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[test]
    fn valid_empty_fast_context_answer_is_explicitly_reported() {
        let temp = tempdir().expect("临时目录应创建成功");
        let response = fast_context::SearchResult {
            files: Vec::new(),
            rg_patterns: vec!["gesture".to_string()],
            file_cache: HashMap::new(),
            stats: fast_context::SearchStats::default(),
            meta: json!({"native": true}),
            answer_received: true,
        };

        let text = format_fast_context_text(
            temp.path().to_str().expect("临时目录路径应为 UTF-8"),
            &response,
            true,
        )
        .expect("合法空 answer 应可格式化");

        assert!(text.contains("No relevant files found."));
        assert!(text.contains("grep keywords: gesture"));
        assert!(text.contains("[fast-context stats]"));
        assert!(!text.contains("Path:"), "合法空 answer 不应伪造代码片段");
    }

    #[test]
    fn degraded_fast_context_errors_trigger_one_independent_retry() {
        assert!(should_retry_fast_context_search(
            "fast-context 未获得合法工具调用: [TOOL_CALLS]..."
        ));
        assert!(should_retry_fast_context_search(
            "fast-context 已达到最大轮次但未获得 answer"
        ));
        assert!(should_retry_fast_context_search(
            "fast-context 返回未知工具调用: readfile"
        ));
        assert!(
            !should_retry_fast_context_search("RATE_LIMITED: Fast Context 当前限流，请稍后重试"),
            "限流类错误不应触发独立兜底重试，避免扩大远端压力"
        );
    }

    #[test]
    fn typed_sections_parse_fast_context_ranges_and_backend() {
        let text = "### sou backend: fast_context\n\nThe following code sections were retrieved:\n\nPath: E:/demo/ui.rs\nLines: L12-L18\nL12:fn render() {}\n\nPath: E:/demo/ui.rs\nLines: L40-L44\nL40:fn audit() {}\n";
        let sections = parse_sou_sections(text, "fast_context");

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].backend, "fast_context");
        assert_eq!(sections[0].location, "E:/demo/ui.rs:12-18");
        assert_eq!(sections[0].excerpt, "L12:fn render() {}");
        assert_eq!(sections[1].location, "E:/demo/ui.rs:40-44");
    }

    #[test]
    fn typed_sections_keep_ace_legacy_locations() {
        let text = "The following code sections were retrieved:\n\nPath: E:/demo/panel.vue (L8-L16)\nconst state = ref(false)\n";
        let sections = parse_sou_sections(text, "ace");

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].backend, "ace");
        assert_eq!(sections[0].location, "E:/demo/panel.vue:8-16");
        assert_eq!(sections[0].excerpt, "const state = ref(false)");
    }

    #[test]
    fn ace_sou_parses_current_markdown_response() {
        let text = r#"## src/rust/mcp/tools/acemcp/mcp.rs
Score: 0.604
Confidence: high
Lines: 27-79

```text
fn create_acemcp_client() {}
Path: this line belongs to the code excerpt
```

## Cargo.toml
Score: 0.418
Confidence: medium
Lines: 49-54

```toml
reqwest = { version = "0.11", features = ["socks"] }
```
"#;

        let sections = parse_sou_sections(text, "ace");

        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections[0].location,
            "src/rust/mcp/tools/acemcp/mcp.rs:27-79"
        );
        assert!(sections[0]
            .excerpt
            .contains("Path: this line belongs to the code excerpt"));
        assert!(!sections[0].excerpt.contains("Score:"));
        assert!(!sections[0].excerpt.contains("```"));
        assert_eq!(sections[1].location, "Cargo.toml:49-54");
        assert!(sections[1].excerpt.contains("reqwest ="));
    }

    #[test]
    fn legacy_auto_order_appends_local_fallback() {
        assert_eq!(
            normalize_auto_order(Some(vec!["fast_context".to_string(), "ace".to_string(),])),
            vec![
                "fast_context".to_string(),
                "ace".to_string(),
                "local".to_string(),
            ]
        );
    }

    #[test]
    fn typed_sections_exclude_backend_diagnostics_from_excerpt() {
        let text = "Path: E:/demo/local.rs\nLines: L2-L3\nL2:fn local_search() {}\n[sou-local] engine=fts5, index_state=ready\n[sou metadata] actual_backend=local\n";
        let sections = parse_sou_sections(text, "local");

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].excerpt, "L2:fn local_search() {}");
    }

    #[tokio::test]
    async fn explicit_local_backend_returns_hits_and_structured_metadata() {
        let temp = tempdir().expect("Local 路由临时项目应创建成功");
        fs::write(
            temp.path().join("local_search.rs"),
            "pub struct LocalIndexStatus;\nfn backend_success_result() {}\n",
        )
        .expect("Local 路由测试源码应写入成功");
        let request = SouRequest {
            project_root_path: temp.path().to_string_lossy().to_string(),
            query: "LocalIndexStatus backendSuccessResult".to_string(),
            backend: Some(BACKEND_LOCAL.to_string()),
            tree_depth: None,
            max_turns: None,
            max_results: Some(5),
            max_commands: None,
            timeout_ms: None,
            exclude_paths: Some(Vec::new()),
        };
        let defaults = FastContextConfig {
            api_key: None,
            tree_depth: 3,
            max_turns: 4,
            max_results: 10,
            max_commands: 8,
            timeout_ms: 30_000,
            exclude_paths: Vec::new(),
        };

        let output = local::search_for_test(
            local::LocalSearchOptions {
                project_root: PathBuf::from(&request.project_root_path),
                query: request.query.clone(),
                max_results: request.max_results.unwrap_or(defaults.max_results) as usize,
                exclude_paths: request
                    .exclude_paths
                    .clone()
                    .unwrap_or_else(|| defaults.exclude_paths.clone()),
                index_dir: temp.path().join("indexes"),
                semantic: local::LocalSemanticSettings {
                    mode: local::LocalSemanticMode::Off,
                    model_dir: crate::mcp::embedding::default_model_dir(),
                    reranker_model_dir: crate::config::default_sou_reranker_model_dir(),
                },
            },
            temp.path().join("route-index.sqlite3"),
        )
        .await
        .expect("Local 路由应完成搜索");
        let result = BackendRunResult {
            backend: BACKEND_LOCAL.to_string(),
            text: output.text,
            hit_count: output.hit_count,
            duration_ms: output.duration_ms,
            engine: Some(output.engine),
            index_state: Some(output.index_state),
            fallback_reason: output.fallback_reason,
            semantic_state: Some(output.semantic_state),
            semantic_model: output.semantic_model,
            semantic_indexed_chunks: Some(output.semantic_indexed_chunks),
            semantic_pending_chunks: Some(output.semantic_pending_chunks),
            semantic_top_score: output.semantic_top_score,
            semantic_mode: Some(output.semantic_mode),
            reranker_state: output.reranker_state,
            reranker_model: output.reranker_model,
            reranker_duration_ms: output.reranker_duration_ms,
            reranker_top_score: output.reranker_top_score,
            fusion: output.fusion,
        };
        assert!(result.hit_count >= 1);
        let call_result = backend_success_result(result, BACKEND_LOCAL, true);
        let metadata = call_result
            .structured_content
            .expect("Local 路由应返回结构化元数据");
        assert_eq!(metadata["actual_backend"], BACKEND_LOCAL);
        assert!(metadata["hit_count"].as_u64().unwrap_or_default() >= 1);
        assert_eq!(metadata["semantic_state"], "disabled");
        assert_eq!(metadata["semantic_mode"], "off");
        assert!(metadata["reranker_state"].is_null());
        assert!(metadata["reranker_model"].is_null());
        assert_eq!(metadata["degraded"], false);
        assert_eq!(call_result.is_error, Some(false));
    }
}
