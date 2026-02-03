// UI/UX Pro Max MCP 工具定义与调用入口

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use serde::Serialize;

use crate::config::load_standalone_config;
use crate::mcp::types::SkillRunRequest;
use crate::{log_important, log_debug};

use super::engine;
use super::localize;
use super::response::{UiuxError, UiuxResponse};
use super::types::{
    UiuxDesignSystemRequest, UiuxLang, UiuxMode, UiuxOutputFormat, UiuxSearchRequest,
    UiuxStackRequest, UiuxSuggestRequest,
};

const DEFAULT_MAX_RESULTS: u32 = 3;

#[derive(Clone, Copy)]
struct UiuxDefaults {
    lang: UiuxLang,
    output_format: UiuxOutputFormat,
    max_results_cap: u32,
    beautify_enabled: bool,
}

impl UiuxDefaults {
    fn load() -> Self {
        // 从配置读取默认值，作为请求缺省兜底
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
        let beautify_enabled = mcp_config
            .and_then(|c| c.uiux_beautify_enabled)
            .unwrap_or(true);

        Self {
            lang,
            output_format,
            max_results_cap,
            beautify_enabled,
        }
    }
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
    // 统一输出 JSON 结构，便于稳定消费
    let response = UiuxResponse::new(tool, lang, data, text, errors);
    let output = serde_json::to_string_pretty(&response)
        .map_err(|e| McpError::internal_error(format!("JSON 序列化失败: {}", e), None))?;
    Ok(CallToolResult::success(vec![Content::text(output)]))
}

#[derive(Serialize)]
struct UiuxSearchData {
    mode: UiuxMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<engine::SearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    beautify: Option<engine::BeautifyResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_text: Option<String>,
}

#[derive(Serialize)]
struct UiuxStackData {
    result: engine::SearchResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_text: Option<String>,
}

#[derive(Serialize)]
struct UiuxDesignSystemData {
    mode: UiuxMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    design_system: Option<engine::DesignSystem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    persisted: Option<engine::PersistSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    beautify: Option<engine::BeautifyResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_text: Option<String>,
}

#[derive(Serialize)]
struct UiuxSuggestData {
    result: engine::SuggestResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_text: Option<String>,
}

/// UI/UX Pro Max MCP 工具
pub struct UiuxTool;

impl UiuxTool {
    pub fn get_tool_definitions() -> Vec<Tool> {
        // 兼容 Antigravity：工具名使用下划线分隔
        let mut tools = Vec::new();

        let search_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索查询" },
                "domain": { "type": "string", "description": "领域（可选）" },
                "max_results": { "type": "number", "description": "最大结果数（可选）" },
                "output_format": { "type": "string", "enum": ["json", "text"], "description": "输出格式（json/text）" },
                "lang": { "type": "string", "enum": ["zh", "en"], "description": "输出语言（zh/en）" },
                "mode": { "type": "string", "enum": ["search", "beautify"], "description": "模式（search/beautify）" }
            },
            "required": ["query"]
        });

        if let serde_json::Value::Object(schema_map) = search_schema {
            tools.push(Tool {
                name: Cow::Borrowed("uiux_search"),
                description: Some(Cow::Borrowed("UI/UX 知识库检索（领域搜索）")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("UI/UX 搜索".to_string()),
            });
        }

        let stack_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索查询" },
                "stack": { "type": "string", "description": "技术栈（必填）" },
                "max_results": { "type": "number", "description": "最大结果数（可选）" },
                "output_format": { "type": "string", "enum": ["json", "text"], "description": "输出格式（json/text）" },
                "lang": { "type": "string", "enum": ["zh", "en"], "description": "输出语言（zh/en）" }
            },
            "required": ["query", "stack"]
        });

        if let serde_json::Value::Object(schema_map) = stack_schema {
            tools.push(Tool {
                name: Cow::Borrowed("uiux_stack"),
                description: Some(Cow::Borrowed("UI/UX 栈相关指南检索")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("UI/UX 栈指南".to_string()),
            });
        }

        let design_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索查询" },
                "project_name": { "type": "string", "description": "项目名称（可选）" },
                "format": { "type": "string", "enum": ["ascii", "markdown"], "description": "输出格式（ascii/markdown）" },
                "persist": { "type": "boolean", "description": "是否写入设计系统文件" },
                "page": { "type": "string", "description": "页面名称（可选）" },
                "output_dir": { "type": "string", "description": "输出目录（可选）" },
                "output_format": { "type": "string", "enum": ["json", "text"], "description": "输出格式（json/text）" },
                "lang": { "type": "string", "enum": ["zh", "en"], "description": "输出语言（zh/en）" },
                "mode": { "type": "string", "enum": ["design_system", "beautify"], "description": "模式（design_system/beautify）" }
            },
            "required": ["query"]
        });

        if let serde_json::Value::Object(schema_map) = design_schema {
            tools.push(Tool {
                name: Cow::Borrowed("uiux_design_system"),
                description: Some(Cow::Borrowed("生成完整设计系统建议")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("UI/UX 设计系统".to_string()),
            });
        }

        let suggest_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "待分析的用户输入" },
                "output_format": { "type": "string", "enum": ["json", "text"], "description": "输出格式（json/text）" },
                "lang": { "type": "string", "enum": ["zh", "en"], "description": "输出语言（zh/en）" }
            },
            "required": ["text"]
        });

        if let serde_json::Value::Object(schema_map) = suggest_schema {
            tools.push(Tool {
                name: Cow::Borrowed("uiux_suggest"),
                description: Some(Cow::Borrowed("基于资料库关键词判断是否建议使用 UI/UX 技能")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("UI/UX 建议".to_string()),
            });
        }

        tools
    }

    pub async fn call_tool(tool_name: &str, arguments: serde_json::Value) -> Result<CallToolResult, McpError> {
        log_important!(info, "[uiux] 工具调用: tool={}", tool_name);
        log_debug!("[uiux] 参数: {:?}", arguments);

        let start = std::time::Instant::now();
        let defaults = UiuxDefaults::load();
        let result = match tool_name {
            "uiux_search" => {
                let req: UiuxSearchRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                log_debug!("[uiux] search 请求: query={}, domain={:?}, mode={:?}", 
                    req.query, req.domain, req.mode);
                handle_search(req, defaults)
            }
            "uiux_stack" => {
                let req: UiuxStackRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                log_debug!("[uiux] stack 请求: query={}, stack={}", req.query, req.stack);
                handle_stack(req, defaults)
            }
            "uiux_design_system" => {
                let req: UiuxDesignSystemRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                log_debug!("[uiux] design_system 请求: query={}, project={:?}", 
                    req.query, req.project_name);
                handle_design_system(req, defaults)
            }
            "uiux_suggest" => {
                let req: UiuxSuggestRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                log_debug!("[uiux] suggest 请求: text_len={}", req.text.len());
                handle_suggest(req, defaults)
            }
            _ => {
                log_important!(warn, "[uiux] 未知工具: {}", tool_name);
                Err(McpError::invalid_params(format!("未知的工具: {}", tool_name), None))
            }
        };

        log_important!(info, "[uiux] 调用完成: tool={}, elapsed={}ms, is_error={}", 
            tool_name, start.elapsed().as_millis(), result.is_err());
        result
    }

    /// skill_ui-ux-pro-max 入口
    pub async fn call_from_skill(action: &str, request: &SkillRunRequest) -> Result<CallToolResult, McpError> {
        log_important!(info, "[uiux] skill 调用: action={}, query={:?}", action, request.query);
        
        let start = std::time::Instant::now();
        let defaults = UiuxDefaults::load();
        let result = match action {
            "design_system" => {
                let query = request
                    .query
                    .clone()
                    .ok_or_else(|| McpError::invalid_params("缺少 query 参数".to_string(), None))?;
                log_debug!("[uiux] skill design_system: query={}", query);
                let req = UiuxDesignSystemRequest {
                    query,
                    project_name: None,
                    format: None,
                    persist: Some(false),
                    page: None,
                    output_dir: None,
                    output_format: Some(UiuxOutputFormat::Text),
                    lang: None,
                    mode: Some(UiuxMode::DesignSystem),
                };
                handle_design_system(req, defaults)
            }
            "search" | "" => {
                let query = request
                    .query
                    .clone()
                    .ok_or_else(|| McpError::invalid_params("缺少 query 参数".to_string(), None))?;
                log_debug!("[uiux] skill search: query={}", query);
                let req = UiuxSearchRequest {
                    query,
                    domain: None,
                    max_results: None,
                    output_format: Some(UiuxOutputFormat::Text),
                    lang: None,
                    mode: Some(UiuxMode::Search),
                };
                handle_search(req, defaults)
            }
            "custom" => {
                log_debug!("[uiux] skill custom: args={:?}", request.args);
                let options = parse_cli_args(request.args.clone().unwrap_or_default());
                let query = options
                    .query
                    .ok_or_else(|| McpError::invalid_params("缺少 query 参数".to_string(), None))?;

                let output_format = if options.json {
                    UiuxOutputFormat::Json
                } else {
                    UiuxOutputFormat::Text
                };

                if options.design_system {
                    let req = UiuxDesignSystemRequest {
                        query,
                        project_name: options.project_name,
                        format: options.format,
                        persist: Some(options.persist),
                        page: options.page,
                        output_dir: options.output_dir,
                        output_format: Some(output_format),
                        lang: None,
                        mode: Some(UiuxMode::DesignSystem),
                    };
                    let result = handle_design_system(req, defaults);
                    log_important!(info, "[uiux] skill 完成: action={}, duration={}ms, success={}", 
                        action, start.elapsed().as_millis(), result.is_ok());
                    return result;
                }

                if let Some(stack) = options.stack {
                    let req = UiuxStackRequest {
                        query,
                        stack,
                        max_results: options.max_results.map(|v| v as u32),
                        output_format: Some(output_format),
                        lang: None,
                    };
                    let result = handle_stack(req, defaults);
                    log_important!(info, "[uiux] skill 完成: action={}, duration={}ms, success={}", 
                        action, start.elapsed().as_millis(), result.is_ok());
                    return result;
                }

                let req = UiuxSearchRequest {
                    query,
                    domain: options.domain,
                    max_results: options.max_results.map(|v| v as u32),
                    output_format: Some(output_format),
                    lang: None,
                    mode: Some(UiuxMode::Search),
                };
                handle_search(req, defaults)
            }
            _ => {
                log_important!(warn, "[uiux] skill 未知 action: {}", action);
                Err(McpError::invalid_params(format!("未知 action: {}", action), None))
            }
        };
        
        // 记录完成日志
        log_important!(info, "[uiux] skill 完成: action={}, duration={}ms, success={}", 
            action, start.elapsed().as_millis(), result.is_ok());
        result
    }
}

fn handle_search(req: UiuxSearchRequest, defaults: UiuxDefaults) -> Result<CallToolResult, McpError> {
    let lang = resolve_lang(req.lang, defaults);
    let output_format = resolve_output_format(req.output_format, defaults);
    let mode = match req.mode {
        Some(UiuxMode::Beautify) => UiuxMode::Beautify,
        _ => UiuxMode::Search,
    };
    let max_results = engine::cap_max_results(req.max_results, defaults.max_results_cap, DEFAULT_MAX_RESULTS);

    if matches!(mode, UiuxMode::Beautify) {
        if !defaults.beautify_enabled {
            let data = UiuxSearchData {
                mode,
                result: None,
                beautify: None,
                legacy_text: None,
            };
            let text = localize::error_text(lang, "UI 提示词美化已被禁用");
            return build_response(
                "uiux_search",
                lang,
                data,
                text,
                vec![UiuxError::new("beautify_disabled", "UI 提示词美化已被禁用")],
            );
        }

        let beautify = engine::beautify_prompt(&req.query, max_results);
        let data = UiuxSearchData {
            mode,
            result: None,
            beautify: Some(beautify),
            legacy_text: None,
        };
        let text = localize::beautify_summary(lang);
        return build_response("uiux_search", lang, data, text, vec![]);
    }

    let result = engine::search_domain(&req.query, req.domain.as_deref(), Some(max_results));
    let legacy_text = if matches!(output_format, UiuxOutputFormat::Text) {
        // 保留旧版文本输出，方便过渡期对照
        Some(engine::format_search_output(&result))
    } else {
        None
    };
    let text = localize::search_summary(lang, mode, &result);
    let errors = result
        .error
        .as_ref()
        .map(|err| vec![UiuxError::new("search_error", err)])
        .unwrap_or_default();
    let data = UiuxSearchData {
        mode,
        result: Some(result),
        beautify: None,
        legacy_text,
    };
    build_response("uiux_search", lang, data, text, errors)
}

fn handle_stack(req: UiuxStackRequest, defaults: UiuxDefaults) -> Result<CallToolResult, McpError> {
    let lang = resolve_lang(req.lang, defaults);
    let output_format = resolve_output_format(req.output_format, defaults);
    let max_results = engine::cap_max_results(req.max_results, defaults.max_results_cap, DEFAULT_MAX_RESULTS);

    let result = engine::search_stack(&req.query, &req.stack, Some(max_results));
    let legacy_text = if matches!(output_format, UiuxOutputFormat::Text) {
        Some(engine::format_search_output(&result))
    } else {
        None
    };
    let text = localize::stack_summary(lang, &result);
    let errors = result
        .error
        .as_ref()
        .map(|err| vec![UiuxError::new("stack_error", err)])
        .unwrap_or_default();
    let data = UiuxStackData { result, legacy_text };
    build_response("uiux_stack", lang, data, text, errors)
}

fn handle_design_system(
    req: UiuxDesignSystemRequest,
    defaults: UiuxDefaults,
) -> Result<CallToolResult, McpError> {
    let lang = resolve_lang(req.lang, defaults);
    let output_format = resolve_output_format(req.output_format, defaults);
    let mode = match req.mode {
        Some(UiuxMode::Beautify) => UiuxMode::Beautify,
        _ => UiuxMode::DesignSystem,
    };

    if matches!(mode, UiuxMode::Beautify) {
        if !defaults.beautify_enabled {
            let data = UiuxDesignSystemData {
                mode,
                design_system: None,
                persisted: None,
                beautify: None,
                legacy_text: None,
            };
            let text = localize::error_text(lang, "UI 提示词美化已被禁用");
            return build_response(
                "uiux_design_system",
                lang,
                data,
                text,
                vec![UiuxError::new("beautify_disabled", "UI 提示词美化已被禁用")],
            );
        }

        let max_results = engine::cap_max_results(None, defaults.max_results_cap, DEFAULT_MAX_RESULTS);
        let beautify = engine::beautify_prompt(&req.query, max_results);
        let data = UiuxDesignSystemData {
            mode,
            design_system: None,
            persisted: None,
            beautify: Some(beautify),
            legacy_text: None,
        };
        let text = localize::beautify_summary(lang);
        return build_response("uiux_design_system", lang, data, text, vec![]);
    }

    let output_dir = req.output_dir.as_ref().map(PathBuf::from);
    let output = engine::generate_design_system(
        &req.query,
        req.project_name.as_deref(),
        req.format.as_deref(),
        req.persist.unwrap_or(false),
        req.page.as_deref(),
        output_dir.as_deref(),
    )
    .map_err(|e| McpError::internal_error(e, None))?;

    let legacy_text = if matches!(output_format, UiuxOutputFormat::Text) {
        // 保留设计系统旧版文本输出，便于人工核对
        Some(output.formatted.clone())
    } else {
        None
    };
    let text = localize::design_system_summary(
        lang,
        &output.design_system.project_name,
        output.persisted.is_some(),
    );
    let data = UiuxDesignSystemData {
        mode,
        design_system: Some(output.design_system),
        persisted: output.persisted,
        beautify: None,
        legacy_text,
    };
    build_response("uiux_design_system", lang, data, text, vec![])
}

fn handle_suggest(req: UiuxSuggestRequest, defaults: UiuxDefaults) -> Result<CallToolResult, McpError> {
    let lang = resolve_lang(req.lang, defaults);
    let output_format = resolve_output_format(req.output_format, defaults);
    let result = engine::suggest(&req.text);
    let text = localize::suggest_summary(lang, &result);
    // text 输出时保留兼容字段，方便旧消费方按需读取
    let legacy_text = if matches!(output_format, UiuxOutputFormat::Text) {
        Some(text.clone())
    } else {
        None
    };
    let data = UiuxSuggestData {
        result,
        legacy_text,
    };
    build_response("uiux_suggest", lang, data, text, vec![])
}

#[derive(Default)]
struct CliOptions {
    query: Option<String>,
    domain: Option<String>,
    stack: Option<String>,
    max_results: Option<usize>,
    json: bool,
    design_system: bool,
    format: Option<String>,
    persist: bool,
    page: Option<String>,
    project_name: Option<String>,
    output_dir: Option<String>,
}

fn parse_cli_args(args: Vec<String>) -> CliOptions {
    let mut opts = CliOptions::default();
    let mut iter = args.into_iter();
    while let Some(token) = iter.next() {
        match token.as_str() {
            "--domain" | "-d" => {
                if let Some(value) = iter.next() {
                    opts.domain = Some(value);
                }
            }
            "--stack" | "-s" => {
                if let Some(value) = iter.next() {
                    opts.stack = Some(value);
                }
            }
            "--max-results" | "-n" => {
                if let Some(value) = iter.next() {
                    opts.max_results = value.parse::<usize>().ok();
                }
            }
            "--json" => {
                opts.json = true;
            }
            "--design-system" | "-ds" => {
                opts.design_system = true;
            }
            "--project-name" | "-p" => {
                if let Some(value) = iter.next() {
                    opts.project_name = Some(value);
                }
            }
            "--format" | "-f" => {
                if let Some(value) = iter.next() {
                    opts.format = Some(value);
                }
            }
            "--persist" => {
                opts.persist = true;
            }
            "--page" => {
                if let Some(value) = iter.next() {
                    opts.page = Some(value);
                }
            }
            "--output-dir" | "-o" => {
                if let Some(value) = iter.next() {
                    opts.output_dir = Some(value);
                }
            }
            _ => {
                if opts.query.is_none() {
                    opts.query = Some(token);
                }
            }
        }
    }
    opts
}
