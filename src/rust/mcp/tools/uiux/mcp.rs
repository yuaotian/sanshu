// UI/UX MCP 工具定义与调用入口
// 新协议目标：单一 uiux 工具，知识库默认走本地 BM25 + BGE 混合检索；
// fast-context 仅在显式请求时用于 A/B 诊断，项目上下文仍通过 sou 检索用户项目。

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use serde::Serialize;

use crate::config::load_standalone_config;
use crate::mcp::tools::sou::{fast_context_key_detected, SouRequest, SouSection};
use crate::mcp::tools::SouTool;
use crate::{log_debug, log_important};

use super::knowledge_base;
use super::localize;
use super::response::{UiuxError, UiuxResponse};
use super::semantic_search;
use super::structured_search;
use super::types::{UiuxAction, UiuxKnowledgeBackend, UiuxLang, UiuxOutputFormat, UiuxRequest};

const DEFAULT_MAX_RESULTS: u32 = 3;
// fast-context 定向知识检索的收敛参数：物化目录只有单个结构化导出文件，
// 压缩树深/轮数/命令数以降低延迟与远端配额消耗
const KB_FAST_CONTEXT_TREE_DEPTH: u8 = 2;
const KB_FAST_CONTEXT_MAX_TURNS: u8 = 2;
const KB_FAST_CONTEXT_MAX_COMMANDS: u8 = 6;

#[derive(Clone, Copy)]
struct UiuxDefaults {
    lang: UiuxLang,
    output_format: UiuxOutputFormat,
    max_results_cap: u32,
    knowledge_backend: UiuxKnowledgeBackend,
}

impl UiuxDefaults {
    fn load() -> Self {
        let config = load_standalone_config().ok();
        let mcp_config = config.as_ref().map(|c| &c.mcp_config);
        let lang = mcp_config
            .and_then(|c| c.uiux_default_lang.as_deref())
            .and_then(parse_lang)
            .unwrap_or(UiuxLang::Zh);
        let output_format = mcp_config
            .and_then(|c| c.uiux_output_format.as_deref())
            .and_then(parse_output_format)
            .unwrap_or(UiuxOutputFormat::Json);
        let max_results_cap = mcp_config
            .and_then(|c| c.uiux_max_results_cap)
            .unwrap_or(10)
            .max(1);
        // 知识检索后端：默认 auto 使用本地 BM25/BGE 混合检索，模型未就绪时保底 BM25。
        let knowledge_backend = mcp_config
            .and_then(|c| c.uiux_knowledge_backend.as_deref())
            .and_then(parse_knowledge_backend)
            .unwrap_or(UiuxKnowledgeBackend::Auto);

        Self {
            lang,
            output_format,
            max_results_cap,
            knowledge_backend,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct UiuxSnippet {
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    excerpt: String,
}

#[derive(Debug, Clone, Serialize)]
struct UiuxQueries {
    knowledge_query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_context_query: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UiuxKnowledgeDiagnostics {
    engine: String,
    version: String,
    status: String,
    domains: Vec<String>,
    rewritten_query: String,
    query_rewrites: Vec<String>,
    top_score: f64,
    token_coverage: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_top_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fusion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UiuxRetrieval {
    requested_knowledge_backend: String,
    knowledge_source: String,
    knowledge_diagnostics: UiuxKnowledgeDiagnostics,
    project_context_source: String,
    project_context_enabled: bool,
    project_context_appended: bool,
    degraded: bool,
    knowledge_duration_ms: u128,
    project_context_duration_ms: u128,
    knowledge_hit_count: usize,
    project_context_hit_count: usize,
    queries: UiuxQueries,
    messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UiuxData {
    action: UiuxAction,
    query: String,
    prompt: String,
    uiux_hits: Vec<UiuxSnippet>,
    project_context: Vec<UiuxSnippet>,
    retrieval: UiuxRetrieval,
}

pub struct UiuxTool;

impl UiuxTool {
    pub fn get_tool_definitions() -> Vec<Tool> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "UI/UX 需求或页面美化目标" },
                "action": { "type": "string", "enum": ["beautify", "describe", "audit", "design_system"], "description": "动作类型，默认 beautify" },
                "project_root_path": { "type": "string", "description": "项目根目录绝对路径（可选，开启 sou 上下文时建议提供）" },
                "current_file_path": { "type": "string", "description": "当前页面/组件文件路径（可选，用于优先召回当前页面上下文）" },
                "context_query": { "type": "string", "description": "项目上下文检索查询（可选，不传则自动生成）" },
                "append_project_context": { "type": "boolean", "description": "是否追加项目上下文，默认 true" },
                "max_results": { "type": "number", "description": "最大返回结果数（可选）" },
                "knowledge_backend": { "type": "string", "enum": ["auto", "fast_context", "local"], "description": "知识检索后端（可选）：auto 使用本地 BM25 + BGE，local 使用确定性 BM25，fast_context 仅用于显式 A/B 诊断" },
                "output_format": { "type": "string", "enum": ["json", "text"], "description": "输出格式（兼容字段，当前统一返回 JSON）" },
                "lang": { "type": "string", "enum": ["zh", "en"], "description": "输出语言（zh/en）" }
            },
            "required": ["query"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            vec![Tool {
                name: Cow::Borrowed("uiux"),
                description: Some(Cow::Borrowed("单一 UI/UX 工具：本地混合检索 UI/UX 知识，并通过 sou 追加项目页面上下文，统一生成可直接交给代码型 AI 的 UI 提示词。")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("UI/UX".to_string()),
            }]
        } else {
            Vec::new()
        }
    }

    pub async fn call_tool(
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, McpError> {
        if tool_name != "uiux" {
            return Err(McpError::invalid_params(
                format!("未知的工具: {}", tool_name),
                None,
            ));
        }

        log_important!(info, "[uiux] 工具调用: tool={}", tool_name);
        log_debug!("[uiux] 参数: {:?}", arguments);

        let defaults = UiuxDefaults::load();
        let req: UiuxRequest = serde_json::from_value(arguments)
            .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

        handle_request(req, defaults).await
    }
}

async fn handle_request(
    req: UiuxRequest,
    defaults: UiuxDefaults,
) -> Result<CallToolResult, McpError> {
    let lang = resolve_lang(req.lang, defaults);
    let _output_format = resolve_output_format(req.output_format, defaults);
    let action = req.action.unwrap_or(UiuxAction::Beautify);
    let knowledge_backend = req.knowledge_backend.unwrap_or(defaults.knowledge_backend);
    let max_results = req
        .max_results
        .unwrap_or(DEFAULT_MAX_RESULTS)
        .max(1)
        .min(defaults.max_results_cap);
    let project_context_enabled =
        req.append_project_context.unwrap_or(true) && req.project_root_path.is_some();
    let sou_enabled = sou_enabled();

    let knowledge_query = build_knowledge_query(&req.query, action);
    let project_context_query = if project_context_enabled {
        Some(build_project_context_query(&req, action))
    } else {
        None
    };

    let knowledge_future = async {
        let started = Instant::now();
        let result = collect_knowledge_hits(
            knowledge_backend,
            &req.query,
            &knowledge_query,
            action,
            max_results as usize,
        )
        .await;
        (result, started.elapsed().as_millis())
    };
    let project_future = async {
        let started = Instant::now();
        let result = if sou_enabled {
            if let (Some(project_root_path), Some(project_query)) = (
                req.project_root_path.as_deref(),
                project_context_query.as_ref(),
            ) {
                collect_project_context_hits(
                    project_root_path,
                    project_query,
                    req.current_file_path.as_deref(),
                    max_results as usize,
                )
                .await
            } else {
                SearchOutcome::skipped("项目上下文未启用或缺少 project_root_path", false)
            }
        } else if project_context_enabled {
            SearchOutcome::skipped("sou 未启用，已跳过项目上下文追加", true)
        } else {
            SearchOutcome::skipped("项目上下文未启用或缺少 project_root_path", false)
        };
        (result, started.elapsed().as_millis())
    };
    let ((knowledge_result, knowledge_duration_ms), (project_result, project_context_duration_ms)) =
        tokio::join!(knowledge_future, project_future);

    let mut errors = Vec::new();
    let mut retrieval_messages = Vec::new();
    let knowledge_source = knowledge_result.source.clone();
    let knowledge_diagnostics = knowledge_result
        .knowledge_diagnostics
        .clone()
        .unwrap_or_else(|| local_diagnostics("not_applicable"));
    if let Some(message) = knowledge_result.message.as_ref() {
        retrieval_messages.push(message.clone());
    }
    let project_context_source = project_result.source.clone();
    if let Some(message) = project_result.message.as_ref() {
        retrieval_messages.push(message.clone());
    }

    let degraded = knowledge_result.degraded || project_result.degraded;
    let project_context_appended = !project_result.hits.is_empty();
    let knowledge_hit_count = knowledge_result.hits.len();
    let project_context_hit_count = project_result.hits.len();
    let uiux_hits = knowledge_result.hits;
    if uiux_hits.is_empty() {
        errors.push(UiuxError::new(
            "uiux_knowledge_empty",
            "未检索到可用的 UI/UX 知识片段",
        ));
    }

    let prompt = build_prompt(action, &req.query, &uiux_hits, &project_result.hits);
    let retrieval = UiuxRetrieval {
        requested_knowledge_backend: knowledge_backend.as_str().to_string(),
        knowledge_source,
        knowledge_diagnostics,
        project_context_source,
        project_context_enabled,
        project_context_appended,
        degraded,
        knowledge_duration_ms,
        project_context_duration_ms,
        knowledge_hit_count,
        project_context_hit_count,
        queries: UiuxQueries {
            knowledge_query,
            project_context_query,
        },
        messages: retrieval_messages,
    };
    let data = UiuxData {
        action,
        query: req.query.clone(),
        prompt,
        uiux_hits,
        project_context: project_result.hits,
        retrieval,
    };

    let text = if errors.is_empty() {
        localize::success_summary(lang, action, project_context_appended, degraded)
    } else {
        localize::error_text(lang, "UI/UX 检索未返回知识片段，请检查查询词")
    };

    build_response("uiux", lang, data, text, errors)
}

fn parse_lang(value: &str) -> Option<UiuxLang> {
    match value.trim().to_lowercase().as_str() {
        "zh" => Some(UiuxLang::Zh),
        "en" => Some(UiuxLang::En),
        _ => None,
    }
}

fn parse_output_format(value: &str) -> Option<UiuxOutputFormat> {
    match value.trim().to_lowercase().as_str() {
        "json" => Some(UiuxOutputFormat::Json),
        "text" => Some(UiuxOutputFormat::Text),
        _ => None,
    }
}

fn parse_knowledge_backend(value: &str) -> Option<UiuxKnowledgeBackend> {
    // 与 sou 后端命名保持一致的宽松归一化（兼容连字符写法）
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "auto" => Some(UiuxKnowledgeBackend::Auto),
        "fast_context" | "fastcontext" | "fast" => Some(UiuxKnowledgeBackend::FastContext),
        "local" | "local_markdown" | "local_bm25" => Some(UiuxKnowledgeBackend::Local),
        _ => None,
    }
}

fn resolve_lang(request: Option<UiuxLang>, defaults: UiuxDefaults) -> UiuxLang {
    request.unwrap_or(defaults.lang)
}

fn resolve_output_format(
    request: Option<UiuxOutputFormat>,
    defaults: UiuxDefaults,
) -> UiuxOutputFormat {
    request.unwrap_or(defaults.output_format)
}

fn build_response<T: Serialize>(
    tool: &str,
    lang: UiuxLang,
    data: T,
    text: String,
    errors: Vec<UiuxError>,
) -> Result<CallToolResult, McpError> {
    let response = UiuxResponse::new(tool, lang, data, text, errors);
    let output = serde_json::to_string_pretty(&response)
        .map_err(|e| McpError::internal_error(format!("JSON 序列化失败: {}", e), None))?;
    Ok(CallToolResult::success(vec![Content::text(output)]))
}

#[derive(Debug)]
struct SearchOutcome {
    source: String,
    hits: Vec<UiuxSnippet>,
    degraded: bool,
    message: Option<String>,
    knowledge_diagnostics: Option<UiuxKnowledgeDiagnostics>,
}

impl SearchOutcome {
    fn skipped(message: &str, degraded: bool) -> Self {
        Self {
            source: "skipped".to_string(),
            hits: Vec::new(),
            degraded,
            message: Some(message.to_string()),
            knowledge_diagnostics: None,
        }
    }
}

/// 知识检索主链路：
/// 1) auto 尝试本地 BM25 + BGE 混合检索，模型未就绪时在 2 秒预算内稳定保底 BM25；
/// 2) local 固定使用 v2.15.0 本地结构化 BM25，作为可复现 A/B 基线；
/// 3) 只有显式 fast_context 才物化同源知识库并执行 A/B 诊断；
/// 4) 显式远端失败时回落本地，同时保留可区分的状态和消息。
async fn collect_knowledge_hits(
    knowledge_backend: UiuxKnowledgeBackend,
    local_query: &str,
    remote_query: &str,
    action: UiuxAction,
    max_results: usize,
) -> SearchOutcome {
    match knowledge_backend {
        UiuxKnowledgeBackend::Auto => {
            let outcome = semantic_search::search(local_query, action, max_results).await;
            return local_hybrid_outcome(outcome);
        }
        UiuxKnowledgeBackend::Local => {
            return local_structured_outcome(
                local_query,
                action,
                max_results,
                false,
                "matched",
                "请求或配置指定 knowledge_backend=local，使用本地结构化 BM25 A/B 基线".to_string(),
            );
        }
        UiuxKnowledgeBackend::FastContext
            if !should_use_fast_context(knowledge_backend, fast_context_key_detected()) =>
        {
            return local_structured_outcome(
                local_query,
                action,
                max_results,
                true,
                "fast_context_unavailable_fallback",
                "本地未检测到 fast-context API Key，已回落到本地结构化 BM25 检索".to_string(),
            );
        }
        UiuxKnowledgeBackend::FastContext => {}
    }

    match search_knowledge_via_fast_context(remote_query, max_results).await {
        Ok(hits) if !hits.is_empty() => SearchOutcome {
            source: "fast_context_kb".to_string(),
            hits,
            degraded: false,
            message: Some("fast-context 已在物化知识库目录完成定向检索".to_string()),
            knowledge_diagnostics: Some(UiuxKnowledgeDiagnostics {
                engine: "fast_context".to_string(),
                version: structured_search::KNOWLEDGE_VERSION.to_string(),
                status: "matched".to_string(),
                domains: Vec::new(),
                rewritten_query: remote_query.to_string(),
                query_rewrites: Vec::new(),
                top_score: 0.0,
                token_coverage: 0.0,
                semantic_model: None,
                semantic_state: None,
                semantic_top_score: None,
                fusion: None,
            }),
        },
        Ok(_) => local_structured_outcome(
            local_query,
            action,
            max_results,
            true,
            "fast_context_empty_fallback",
            "fast-context 知识检索无命中，已回落到本地结构化 BM25 检索".to_string(),
        ),
        Err(err) => local_structured_outcome(
            local_query,
            action,
            max_results,
            true,
            "fast_context_error_fallback",
            format!(
                "fast-context 知识检索失败，已回落到本地结构化 BM25 检索：{}",
                err
            ),
        ),
    }
}

fn local_hybrid_outcome(outcome: semantic_search::HybridSearchOutcome) -> SearchOutcome {
    let report = outcome.report;
    let mut status = if outcome.semantic_state == "ready" {
        "matched_hybrid".to_string()
    } else {
        format!("matched_bm25_semantic_{}", outcome.semantic_state)
    };
    let mut message = outcome.message;
    if report.abstained {
        status.push_str("_low_confidence");
        message.push_str("；本地检索因低置信度拒答");
    }
    let source = if outcome.semantic_state == "ready" {
        "local_hybrid"
    } else {
        "local_bm25"
    };
    SearchOutcome {
        source: source.to_string(),
        hits: report
            .hits
            .into_iter()
            .map(|hit| UiuxSnippet {
                source: hit.source,
                location: Some(hit.location),
                excerpt: hit.excerpt,
            })
            .collect(),
        degraded: false,
        message: Some(message),
        knowledge_diagnostics: Some(UiuxKnowledgeDiagnostics {
            engine: outcome.engine,
            version: structured_search::KNOWLEDGE_VERSION.to_string(),
            status,
            domains: report.domains,
            rewritten_query: report.rewritten_query,
            query_rewrites: report.query_rewrites,
            top_score: report.top_score,
            token_coverage: report.token_coverage,
            semantic_model: Some(super::model_manager::MODEL_NAME.to_string()),
            semantic_state: Some(outcome.semantic_state),
            semantic_top_score: outcome.semantic_top_score,
            fusion: outcome.fusion,
        }),
    }
}

/// 本地结构化检索结果，统一承载显式本地、auto 主链和远端回落三种状态。
fn local_structured_outcome(
    query: &str,
    action: UiuxAction,
    max_results: usize,
    degraded: bool,
    status: &str,
    mut message: String,
) -> SearchOutcome {
    let report = structured_search::search(query, action, max_results);
    let status = if report.abstained {
        message.push_str("；本地 BM25 因低置信度拒答");
        format!("{}_low_confidence", status)
    } else {
        status.to_string()
    };

    SearchOutcome {
        source: "local_bm25".to_string(),
        hits: report
            .hits
            .into_iter()
            .map(|hit| UiuxSnippet {
                source: hit.source,
                location: Some(hit.location),
                excerpt: hit.excerpt,
            })
            .collect(),
        degraded,
        message: Some(message),
        knowledge_diagnostics: Some(UiuxKnowledgeDiagnostics {
            engine: structured_search::KNOWLEDGE_ENGINE.to_string(),
            version: structured_search::KNOWLEDGE_VERSION.to_string(),
            status,
            domains: report.domains,
            rewritten_query: report.rewritten_query,
            query_rewrites: report.query_rewrites,
            top_score: report.top_score,
            token_coverage: report.token_coverage,
            semantic_model: None,
            semantic_state: None,
            semantic_top_score: None,
            fusion: None,
        }),
    }
}

/// 纯函数隔离后端选择规则，避免单测依赖本机配置或真实远端服务。
fn should_use_fast_context(backend: UiuxKnowledgeBackend, fast_context_key_detected: bool) -> bool {
    backend == UiuxKnowledgeBackend::FastContext && fast_context_key_detected
}

fn local_diagnostics(status: &str) -> UiuxKnowledgeDiagnostics {
    UiuxKnowledgeDiagnostics {
        engine: structured_search::KNOWLEDGE_ENGINE.to_string(),
        version: structured_search::KNOWLEDGE_VERSION.to_string(),
        status: status.to_string(),
        domains: Vec::new(),
        rewritten_query: String::new(),
        query_rewrites: Vec::new(),
        top_score: 0.0,
        token_coverage: 0.0,
        semantic_model: None,
        semantic_state: None,
        semantic_top_score: None,
        fusion: None,
    }
}

/// 物化知识库并用 fast-context 后端做定向检索
async fn search_knowledge_via_fast_context(
    query: &str,
    max_results: usize,
) -> Result<Vec<UiuxSnippet>, String> {
    let kb_dir =
        knowledge_base::ensure_materialized().map_err(|e| format!("知识库物化失败: {}", e))?;

    let sections = SouTool::search_sections(SouRequest {
        project_root_path: kb_dir,
        query: query.to_string(),
        backend: Some("fast_context".to_string()),
        tree_depth: Some(KB_FAST_CONTEXT_TREE_DEPTH),
        max_turns: Some(KB_FAST_CONTEXT_MAX_TURNS),
        max_results: Some(max_results.clamp(1, 8) as u8),
        max_commands: Some(KB_FAST_CONTEXT_MAX_COMMANDS),
        timeout_ms: None,
        exclude_paths: None,
    })
    .await
    .map_err(|e| format!("sou 调用失败: {}", e))?;

    // 只保留知识库文件本身的命中，防止意外扫到目录内其他文件
    Ok(sections
        .into_iter()
        .filter(|section| {
            section
                .location
                .contains(knowledge_base::UIUX_MARKDOWN_FILENAME)
        })
        .take(max_results)
        .map(|section| UiuxSnippet {
            source: "fast_context_kb".to_string(),
            location: Some(section.location),
            excerpt: compact_sou_excerpt(&section.excerpt),
        })
        .collect())
}

fn sou_enabled() -> bool {
    load_standalone_config()
        .ok()
        .and_then(|config| config.mcp_config.tools.get("sou").copied())
        .unwrap_or(false)
}

async fn collect_project_context_hits(
    project_root_path: &str,
    query: &str,
    current_file_path: Option<&str>,
    max_results: usize,
) -> SearchOutcome {
    project_context_outcome(
        search_sou_sections(project_root_path, query).await,
        current_file_path,
        max_results,
    )
}

fn project_context_outcome(
    result: Result<Vec<SouSection>, String>,
    current_file_path: Option<&str>,
    max_results: usize,
) -> SearchOutcome {
    match result {
        Ok(mut sections) => {
            sections.retain(|section| is_project_context_candidate(&section.location));
            if let Some(current_file_path) = current_file_path {
                let file_hint = current_file_hint(current_file_path);
                if let Some(file_hint) = file_hint {
                    sections.sort_by(|a, b| {
                        let a_hit = a.location.contains(&file_hint);
                        let b_hit = b.location.contains(&file_hint);
                        b_hit.cmp(&a_hit)
                    });
                }
            }

            let hits: Vec<UiuxSnippet> = sections
                .into_iter()
                .take(max_results)
                .map(|section| UiuxSnippet {
                    source: format!("sou:{}", section.backend),
                    location: Some(section.location),
                    excerpt: compact_sou_excerpt(&section.excerpt),
                })
                .collect();

            if hits.is_empty() {
                return SearchOutcome::skipped("sou 未命中可追加的项目页面上下文", true);
            }

            SearchOutcome {
                source: "sou".to_string(),
                hits,
                degraded: false,
                message: Some("已通过 sou 追加项目页面上下文".to_string()),
                knowledge_diagnostics: None,
            }
        }
        Err(err) => SearchOutcome {
            source: "skipped".to_string(),
            hits: Vec::new(),
            degraded: true,
            message: Some(format!("项目上下文追加失败，已跳过：{}", err)),
            knowledge_diagnostics: None,
        },
    }
}

async fn search_sou_sections(
    project_root_path: &str,
    query: &str,
) -> Result<Vec<SouSection>, String> {
    SouTool::search_sections(SouRequest {
        project_root_path: project_root_path.to_string(),
        query: query.to_string(),
        backend: None,
        tree_depth: None,
        max_turns: None,
        max_results: None,
        max_commands: None,
        timeout_ms: None,
        exclude_paths: None,
    })
    .await
    .map_err(|e| format!("sou 调用失败: {}", e))
}

fn compact_sou_excerpt(excerpt: &str) -> String {
    let excerpt = excerpt
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(24)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    truncate_text(&excerpt, 900)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }

    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn build_knowledge_query(query: &str, action: UiuxAction) -> String {
    // 只保留"需求原文 + 动作主题词"。旧实现还会拼入 "ui-ux-pro-max-skill.md"、
    // "UI/UX Pro Max" 等定位噪声词，导致本地打分偏向命中文档页脚/品牌字样
    // 而非语义相关小节，已移除。
    let mut parts = vec![query.to_string()];
    match action {
        UiuxAction::Beautify => parts.extend(
            [
                "页面美化",
                "style",
                "color",
                "typography",
                "layout",
                "motion",
                "responsive",
            ]
            .into_iter()
            .map(str::to_string),
        ),
        UiuxAction::Describe => parts.extend(
            [
                "UI描述",
                "visual language",
                "style",
                "component",
                "hierarchy",
                "typography",
            ]
            .into_iter()
            .map(str::to_string),
        ),
        UiuxAction::Audit => parts.extend(
            [
                "UI审查",
                "ux",
                "accessibility",
                "spacing",
                "alignment",
                "state",
                "responsive",
            ]
            .into_iter()
            .map(str::to_string),
        ),
        UiuxAction::DesignSystem => parts.extend(
            [
                "设计系统",
                "design system",
                "color",
                "typography",
                "component",
                "token",
                "state",
            ]
            .into_iter()
            .map(str::to_string),
        ),
    }
    join_query_terms(parts)
}

fn build_project_context_query(req: &UiuxRequest, action: UiuxAction) -> String {
    if let Some(context_query) = req.context_query.as_ref() {
        return context_query.clone();
    }

    let mut parts = vec![req.query.clone()];
    if let Some(current_file_path) = req.current_file_path.as_deref() {
        parts.extend(current_file_query_hints(current_file_path));
    }
    match action {
        UiuxAction::Beautify => parts.extend(
            ["页面", "组件", "样式", "布局", "交互", "theme", "class"]
                .into_iter()
                .map(str::to_string),
        ),
        UiuxAction::Describe => parts.extend(
            ["页面", "组件", "视觉", "结构", "布局", "内容区块"]
                .into_iter()
                .map(str::to_string),
        ),
        UiuxAction::Audit => parts.extend(
            ["页面", "组件", "状态", "交互", "可访问性", "响应式"]
                .into_iter()
                .map(str::to_string),
        ),
        UiuxAction::DesignSystem => parts.extend(
            ["页面", "组件", "主题", "颜色", "字体", "变量", "token"]
                .into_iter()
                .map(str::to_string),
        ),
    }
    join_query_terms(parts)
}

fn current_file_hint(current_file_path: &str) -> Option<String> {
    let file_stem = Path::new(current_file_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(file_stem)
}

fn current_file_query_hints(current_file_path: &str) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(file_hint) = current_file_hint(current_file_path) {
        hints.push(file_hint);
    }

    let path = Path::new(current_file_path);
    if let Some(parent) = path
        .parent()
        .and_then(|value| value.file_name())
        .and_then(|value| value.to_str())
    {
        if !parent.trim().is_empty() {
            hints.push(parent.trim().to_string());
        }
    }

    hints
}

fn join_query_terms(parts: Vec<String>) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut output = Vec::new();
    for part in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            output.push(trimmed.to_string());
        }
    }
    output.join(" ")
}

fn build_prompt(
    action: UiuxAction,
    query: &str,
    uiux_hits: &[UiuxSnippet],
    project_hits: &[UiuxSnippet],
) -> String {
    let mut sections = Vec::new();
    sections.push("# 角色".to_string());
    sections.push("你是资深 UI/UX 设计与前端改造助手。".to_string());
    sections.push("# 任务".to_string());
    sections.push(format!("- 当前需求：{}", query));
    sections.push(format!("- 动作：{}", action.as_str()));
    sections.push("# 硬约束".to_string());
    sections.push("- 严格遵循 KISS / YAGNI / SOLID。".to_string());
    sections.push("- 不擅自修改业务流程与数据语义。".to_string());
    sections.push("- 输出中文，且要可直接发给代码型 AI。".to_string());

    if !project_hits.is_empty() {
        sections.push("# 项目上下文".to_string());
        sections.push(render_snippets(project_hits));
    }

    if !uiux_hits.is_empty() {
        sections.push("# UI/UX 参考知识".to_string());
        sections.push(render_snippets(uiux_hits));
    }

    sections.push("# 输出要求".to_string());
    let action_instruction = match action {
        UiuxAction::Beautify => {
            "请输出一段“页面美化提示词”，必须依次包含：\n1. 视觉方向与风格关键词\n2. 布局与信息层级调整\n3. 关键组件（按钮/卡片/表单/导航等）改造要点\n4. 配色、字体、间距、圆角、阴影、动效要求\n5. 响应式与状态（hover/focus/disabled/loading）约束\n6. 禁止事项（不要破坏业务结构、不要引入与现有上下文冲突的风格）"
        }
        UiuxAction::Describe => {
            "请输出一段“UI 描述提示词”，必须依次包含：\n1. 页面整体气质\n2. 视觉语言关键词\n3. 配色与字体性格\n4. 组件触感与交互反馈\n5. 页面氛围与品牌感\n6. 不适合采用的反向风格"
        }
        UiuxAction::Audit => {
            "请输出一段“UI 审查提示词”，要求 AI 围绕：可访问性、对齐、间距、层级、状态一致性、视觉噪音、移动端适配、交互反馈进行审查，并按严重级别输出问题与改进建议。"
        }
        UiuxAction::DesignSystem => {
            "请输出一段“设计系统提示词”，必须覆盖：颜色 token、字体与字号层级、间距、圆角、阴影、按钮状态、表单状态、卡片语义、导航规范、响应式规则，以及组件复用约束。"
        }
    };
    sections.push(action_instruction.to_string());
    sections.push("# 输出风格".to_string());
    sections.push("只输出最终提示词正文，不要解释你的推理过程，不要写额外前言。".to_string());
    sections.join("\n\n")
}

fn render_snippets(snippets: &[UiuxSnippet]) -> String {
    snippets
        .iter()
        .enumerate()
        .map(|(index, snippet)| {
            let location = snippet.location.as_deref().unwrap_or("未知位置");
            format!(
                "片段 {} [{}]\n{}\n{}",
                index + 1,
                snippet.source,
                location,
                snippet.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn is_project_context_candidate(location: &str) -> bool {
    let normalized = sou_location_path(location)
        .to_lowercase()
        .replace('\\', "/");
    if normalized.contains(knowledge_base::UIUX_MARKDOWN_FILENAME)
        || normalized.ends_with(".md")
        || normalized.contains("/rules/")
        || normalized.ends_with("readme.md")
        || normalized.contains("/skills/")
    {
        return false;
    }

    normalized.ends_with(".vue")
        || normalized.ends_with(".tsx")
        || normalized.ends_with(".jsx")
        || normalized.ends_with(".ts")
        || normalized.ends_with(".js")
        || normalized.ends_with(".css")
        || normalized.ends_with(".scss")
        || normalized.ends_with(".html")
        || normalized.ends_with(".rs")
}

fn sou_location_path(location: &str) -> &str {
    let Some((path, suffix)) = location.rsplit_once(':') else {
        return location;
    };
    if suffix.contains('-') && suffix.chars().all(|ch| ch.is_ascii_digit() || ch == '-') {
        path
    } else {
        location
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_backend_parser_accepts_documented_values() {
        assert_eq!(
            parse_knowledge_backend("auto"),
            Some(UiuxKnowledgeBackend::Auto)
        );
        assert_eq!(
            parse_knowledge_backend("fast-context"),
            Some(UiuxKnowledgeBackend::FastContext)
        );
        assert_eq!(
            parse_knowledge_backend("local"),
            Some(UiuxKnowledgeBackend::Local)
        );
    }

    #[test]
    fn local_backend_never_uses_fast_context() {
        assert!(!should_use_fast_context(UiuxKnowledgeBackend::Local, true));
    }

    #[test]
    fn forced_fast_context_only_requires_detected_key() {
        assert!(should_use_fast_context(
            UiuxKnowledgeBackend::FastContext,
            true
        ));
        assert!(!should_use_fast_context(
            UiuxKnowledgeBackend::FastContext,
            false
        ));
    }

    #[test]
    fn auto_backend_never_uses_fast_context() {
        assert!(!should_use_fast_context(UiuxKnowledgeBackend::Auto, true));
        assert!(!should_use_fast_context(UiuxKnowledgeBackend::Auto, false));
    }

    #[test]
    fn explicit_local_outcome_is_not_degraded() {
        let outcome = local_structured_outcome(
            "仪表盘 配色",
            UiuxAction::Beautify,
            1,
            false,
            "matched",
            "本地 A/B 基线".to_string(),
        );
        assert_eq!(outcome.source, "local_bm25");
        assert!(!outcome.degraded);
        assert!(!outcome.hits.is_empty());
        assert_eq!(
            outcome
                .knowledge_diagnostics
                .as_ref()
                .map(|diagnostics| diagnostics.version.as_str()),
            Some("v2.15.0")
        );
    }

    #[test]
    fn project_context_failure_and_empty_candidates_are_degraded() {
        let failed = project_context_outcome(Err("backend error".to_string()), None, 3);
        assert!(failed.degraded);
        assert_eq!(failed.source, "skipped");

        let filtered = project_context_outcome(
            Ok(vec![SouSection {
                backend: "ace".to_string(),
                location: "E:/demo/README.md:1-10".to_string(),
                excerpt: "说明".to_string(),
            }]),
            None,
            3,
        );
        assert!(filtered.degraded);
        assert!(filtered.hits.is_empty());
    }

    #[test]
    fn project_context_success_preserves_backend_and_location() {
        let outcome = project_context_outcome(
            Ok(vec![SouSection {
                backend: "fast_context".to_string(),
                location: "E:/demo/Panel.vue:12-24".to_string(),
                excerpt: "const open = ref(false)".to_string(),
            }]),
            Some("Panel.vue"),
            3,
        );

        assert!(!outcome.degraded);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].source, "sou:fast_context");
        assert_eq!(
            outcome.hits[0].location.as_deref(),
            Some("E:/demo/Panel.vue:12-24")
        );
    }
}
