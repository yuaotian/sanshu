// UI/UX Pro Max MCP 工具定义与调用入口

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};

use crate::mcp::types::SkillRunRequest;

use super::engine;
use super::types::{UiuxDesignSystemRequest, UiuxSearchRequest, UiuxStackRequest, UiuxSuggestRequest};

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
                "format": { "type": "string", "enum": ["text", "json"], "description": "输出格式（text/json）" }
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
                "format": { "type": "string", "enum": ["text", "json"], "description": "输出格式（text/json）" }
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
                "output_dir": { "type": "string", "description": "输出目录（可选）" }
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
                "text": { "type": "string", "description": "待分析的用户输入" }
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
        match tool_name {
            "uiux_search" => {
                let req: UiuxSearchRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                let result = engine::search_domain(&req.query, req.domain.as_deref(), req.max_results.map(|v| v as usize));
                let format = req.format.as_deref().unwrap_or("text");
                let output = if format == "json" {
                    engine::format_search_json(&result)
                        .map_err(|e| McpError::internal_error(e, None))?
                } else {
                    engine::format_search_output(&result)
                };
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "uiux_stack" => {
                let req: UiuxStackRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                let result = engine::search_stack(&req.query, &req.stack, req.max_results.map(|v| v as usize));
                let format = req.format.as_deref().unwrap_or("text");
                let output = if format == "json" {
                    engine::format_search_json(&result)
                        .map_err(|e| McpError::internal_error(e, None))?
                } else {
                    engine::format_search_output(&result)
                };
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "uiux_design_system" => {
                let req: UiuxDesignSystemRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
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
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "uiux_suggest" => {
                let req: UiuxSuggestRequest = serde_json::from_value(arguments)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;
                let result = engine::suggest(&req.text);
                let output = serde_json::to_string_pretty(&result)
                    .map_err(|e| McpError::internal_error(format!("JSON 序列化失败: {}", e), None))?;
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            _ => Err(McpError::invalid_params(format!("未知的工具: {}", tool_name), None)),
        }
    }

    /// skill_ui-ux-pro-max 入口
    pub async fn call_from_skill(action: &str, request: &SkillRunRequest) -> Result<CallToolResult, McpError> {
        match action {
            "design_system" => {
                let query = request
                    .query
                    .clone()
                    .ok_or_else(|| McpError::invalid_params("缺少 query 参数".to_string(), None))?;
                let output = engine::generate_design_system(&query, None, None, false, None, None)
                    .map_err(|e| McpError::internal_error(e, None))?;
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            "search" | "" => {
                let query = request
                    .query
                    .clone()
                    .ok_or_else(|| McpError::invalid_params("缺少 query 参数".to_string(), None))?;
                let result = engine::search_domain(&query, None, None);
                Ok(CallToolResult::success(vec![Content::text(engine::format_search_output(&result))]))
            }
            "custom" => {
                let options = parse_cli_args(request.args.clone().unwrap_or_default());
                let query = options
                    .query
                    .ok_or_else(|| McpError::invalid_params("缺少 query 参数".to_string(), None))?;

                if options.design_system {
                    let output_dir = options.output_dir.map(PathBuf::from);
                    let output = engine::generate_design_system(
                        &query,
                        options.project_name.as_deref(),
                        options.format.as_deref(),
                        options.persist,
                        options.page.as_deref(),
                        output_dir.as_deref(),
                    )
                    .map_err(|e| McpError::internal_error(e, None))?;
                    return Ok(CallToolResult::success(vec![Content::text(output)]));
                }

                let result = if let Some(stack) = options.stack.as_deref() {
                    engine::search_stack(&query, stack, options.max_results)
                } else {
                    engine::search_domain(&query, options.domain.as_deref(), options.max_results)
                };

                let output = if options.json {
                    engine::format_search_json(&result)
                        .map_err(|e| McpError::internal_error(e, None))?
                } else {
                    engine::format_search_output(&result)
                };
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            _ => Err(McpError::invalid_params(format!("未知 action: {}", action), None)),
        }
    }
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
