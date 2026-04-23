// UI/UX MCP 工具定义与调用入口
// 新协议目标：单一 uiux 工具，内部统一编排 sou 检索与本地 markdown 降级。

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use serde::Serialize;

use crate::config::load_standalone_config;
use crate::mcp::tools::acemcp::types::AcemcpRequest;
use crate::mcp::tools::AcemcpTool;
use crate::{log_debug, log_important};

use super::localize;
use super::markdown_search;
use super::response::{UiuxError, UiuxResponse};
use super::types::{UiuxAction, UiuxLang, UiuxOutputFormat, UiuxRequest};

const DEFAULT_MAX_RESULTS: u32 = 3;
const UIUX_MARKDOWN_FILENAME: &str = "ui-ux-pro-max-skill.md";
const UIUX_MARKDOWN_PATH: &str = "src/rust/assets/resources/ui-ux-pro-max-skill.md";

#[derive(Clone, Copy)]
struct UiuxDefaults {
    lang: UiuxLang,
    output_format: UiuxOutputFormat,
    max_results_cap: u32,
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

        Self {
            lang,
            output_format,
            max_results_cap,
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
struct UiuxRetrieval {
    knowledge_source: String,
    project_context_source: String,
    project_context_enabled: bool,
    degraded: bool,
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

#[derive(Debug, Clone)]
struct SouSection {
    location: String,
    excerpt: String,
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
                "output_format": { "type": "string", "enum": ["json", "text"], "description": "输出格式（兼容字段，当前统一返回 JSON）" },
                "lang": { "type": "string", "enum": ["zh", "en"], "description": "输出语言（zh/en）" }
            },
            "required": ["query"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            vec![Tool {
                name: Cow::Borrowed("uiux"),
                description: Some(Cow::Borrowed("单一 UI/UX 工具：优先通过 sou 检索项目页面与 UI/UX 资料，并在 sou 不可用时回退到本地 markdown 检索，统一生成可直接喂给 AI 的 UI 提示词。")),
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

    pub async fn call_tool(tool_name: &str, arguments: serde_json::Value) -> Result<CallToolResult, McpError> {
        if tool_name != "uiux" {
            return Err(McpError::invalid_params(format!("未知的工具: {}", tool_name), None));
        }

        log_important!(info, "[uiux] 工具调用: tool={}", tool_name);
        log_debug!("[uiux] 参数: {:?}", arguments);

        let defaults = UiuxDefaults::load();
        let req: UiuxRequest = serde_json::from_value(arguments)
            .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

        handle_request(req, defaults).await
    }
}

async fn handle_request(req: UiuxRequest, defaults: UiuxDefaults) -> Result<CallToolResult, McpError> {
    let lang = resolve_lang(req.lang, defaults);
    let _output_format = resolve_output_format(req.output_format, defaults);
    let action = req.action.unwrap_or(UiuxAction::Beautify);
    let max_results = req.max_results.unwrap_or(DEFAULT_MAX_RESULTS).max(1).min(defaults.max_results_cap);
    let project_context_enabled = req.append_project_context.unwrap_or(true) && req.project_root_path.is_some();
    let sou_enabled = sou_enabled();

    let knowledge_query = build_knowledge_query(&req.query, action);
    let project_context_query = if project_context_enabled {
        Some(build_project_context_query(&req, action))
    } else {
        None
    };

    let mut errors = Vec::new();
    let mut retrieval_messages = Vec::new();
    let mut degraded = false;

    let knowledge_result = collect_knowledge_hits(
        sou_enabled,
        req.project_root_path.as_deref(),
        &knowledge_query,
        max_results as usize,
    )
    .await;
    let knowledge_source = knowledge_result.source.clone();
    if let Some(message) = knowledge_result.message.as_ref() {
        retrieval_messages.push(message.clone());
    }
    if knowledge_result.degraded {
        degraded = true;
    }

    let project_result = if sou_enabled {
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
            SearchOutcome::skipped("项目上下文未启用或缺少 project_root_path")
        }
    } else {
        SearchOutcome::skipped("sou 未启用，已跳过项目上下文追加")
    };
    let project_context_source = project_result.source.clone();
    if let Some(message) = project_result.message.as_ref() {
        retrieval_messages.push(message.clone());
    }

    let uiux_hits = knowledge_result.hits;
    if uiux_hits.is_empty() {
        errors.push(UiuxError::new(
            "uiux_knowledge_empty",
            "未检索到可用的 UI/UX 知识片段",
        ));
    }

    let prompt = build_prompt(action, &req.query, &uiux_hits, &project_result.hits);
    let retrieval = UiuxRetrieval {
        knowledge_source,
        project_context_source,
        project_context_enabled,
        degraded,
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
        localize::success_summary(lang, action, project_context_enabled, degraded)
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
}

impl SearchOutcome {
    fn skipped(message: &str) -> Self {
        Self {
            source: "skipped".to_string(),
            hits: Vec::new(),
            degraded: false,
            message: Some(message.to_string()),
        }
    }
}

async fn collect_knowledge_hits(
    sou_enabled: bool,
    project_root_path: Option<&str>,
    query: &str,
    max_results: usize,
) -> SearchOutcome {
    if !sou_enabled {
        return SearchOutcome {
            source: "local_markdown".to_string(),
            hits: markdown_search::search_markdown(query, max_results)
                .into_iter()
                .map(|hit| UiuxSnippet {
                    source: hit.source,
                    location: Some(hit.location),
                    excerpt: hit.excerpt,
                })
                .collect(),
            degraded: true,
            message: Some("sou 未启用，已直接使用本地 markdown 检索".to_string()),
        };
    }

    if let Some(project_root_path) = project_root_path {
        match search_sou_sections(project_root_path, query).await {
            Ok(sections) => {
                let hits: Vec<UiuxSnippet> = sections
                    .into_iter()
                    .filter(|section| section.location.contains(UIUX_MARKDOWN_FILENAME))
                    .take(max_results)
                    .map(|section| UiuxSnippet {
                        source: "sou".to_string(),
                        location: Some(section.location),
                        excerpt: section.excerpt,
                    })
                    .collect();

                if !hits.is_empty() {
                    return SearchOutcome {
                        source: "sou".to_string(),
                        hits,
                        degraded: false,
                        message: Some(format!("sou 已命中 {}", UIUX_MARKDOWN_PATH)),
                    };
                }

                return SearchOutcome {
                    source: "local_markdown".to_string(),
                    hits: markdown_search::search_markdown(query, max_results)
                        .into_iter()
                        .map(|hit| UiuxSnippet {
                            source: hit.source,
                            location: Some(hit.location),
                            excerpt: hit.excerpt,
                        })
                        .collect(),
                    degraded: true,
                    message: Some(format!(
                        "sou 未命中 {}，已降级到本地 markdown 检索",
                        markdown_search::source_path()
                    )),
                };
            }
            Err(err) => {
                return SearchOutcome {
                    source: "local_markdown".to_string(),
                    hits: markdown_search::search_markdown(query, max_results)
                        .into_iter()
                        .map(|hit| UiuxSnippet {
                            source: hit.source,
                            location: Some(hit.location),
                            excerpt: hit.excerpt,
                        })
                        .collect(),
                    degraded: true,
                    message: Some(format!("sou 知识检索失败，已降级到本地 markdown：{}", err)),
                };
            }
        }
    }

    SearchOutcome {
        source: "local_markdown".to_string(),
        hits: markdown_search::search_markdown(query, max_results)
            .into_iter()
            .map(|hit| UiuxSnippet {
                source: hit.source,
                location: Some(hit.location),
                excerpt: hit.excerpt,
            })
            .collect(),
        degraded: true,
        message: Some("未提供 project_root_path，已直接使用本地 markdown 检索".to_string()),
    }
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
    match search_sou_sections(project_root_path, query).await {
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

            let hits = sections
                .into_iter()
                .take(max_results)
                .map(|section| UiuxSnippet {
                    source: "sou".to_string(),
                    location: Some(section.location),
                    excerpt: section.excerpt,
                })
                .collect();

            SearchOutcome {
                source: "sou".to_string(),
                hits,
                degraded: false,
                message: Some("已通过 sou 追加项目页面上下文".to_string()),
            }
        }
        Err(err) => SearchOutcome {
            source: "skipped".to_string(),
            hits: Vec::new(),
            degraded: false,
            message: Some(format!("项目上下文追加失败，已跳过：{}", err)),
        },
    }
}

async fn search_sou_sections(project_root_path: &str, query: &str) -> Result<Vec<SouSection>, String> {
    let result = AcemcpTool::search_context(AcemcpRequest {
        project_root_path: project_root_path.to_string(),
        query: query.to_string(),
    })
    .await
    .map_err(|e| format!("sou 调用失败: {}", e))?;

    let text = extract_call_result_text(&result);
    if result.is_error.unwrap_or(false) || is_sou_error_text(&text) {
        return Err(text);
    }

    let sections = parse_sou_sections(&text);
    if sections.is_empty() {
        Err("sou 未返回可解析的代码片段".to_string())
    } else {
        Ok(sections)
    }
}

fn extract_call_result_text(result: &CallToolResult) -> String {
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

fn is_sou_error_text(text: &str) -> bool {
    let normalized = text.trim();
    normalized.starts_with("Acemcp搜索失败:")
        || normalized.starts_with("搜索失败:")
        || normalized.starts_with("索引更新失败:")
        || normalized.starts_with("代码搜索失败:")
}

fn parse_sou_sections(text: &str) -> Vec<SouSection> {
    let mut sections = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if let Some(path) = line.strip_prefix("Path: ") {
            flush_sou_section(&mut sections, &mut current_path, &mut current_lines);
            current_path = Some(path.trim().to_string());
            continue;
        }

        if line.starts_with("The following code sections were retrieved:") {
            continue;
        }

        if current_path.is_some() {
            current_lines.push(line.trim_end().to_string());
        }
    }

    flush_sou_section(&mut sections, &mut current_path, &mut current_lines);
    sections
}

fn flush_sou_section(
    sections: &mut Vec<SouSection>,
    current_path: &mut Option<String>,
    current_lines: &mut Vec<String>,
) {
    let Some(path) = current_path.take() else {
        current_lines.clear();
        return;
    };

    let excerpt = current_lines
        .iter()
        .filter(|line| !line.trim().is_empty() && line.trim() != "...")
        .take(24)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let excerpt = truncate_text(&excerpt, 900);
    if !excerpt.trim().is_empty() {
        sections.push(SouSection {
            location: path,
            excerpt,
        });
    }
    current_lines.clear();
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
    let mut parts = vec![
        UIUX_MARKDOWN_FILENAME.to_string(),
        "UI/UX Pro Max".to_string(),
        query.to_string(),
    ];
    match action {
        UiuxAction::Beautify => parts.extend(
            ["页面美化", "style", "color", "typography", "layout", "motion", "responsive"]
                .into_iter()
                .map(str::to_string),
        ),
        UiuxAction::Describe => parts.extend(
            ["UI描述", "visual language", "style", "component", "hierarchy", "typography"]
                .into_iter()
                .map(str::to_string),
        ),
        UiuxAction::Audit => parts.extend(
            ["UI审查", "ux", "accessibility", "spacing", "alignment", "state", "responsive"]
                .into_iter()
                .map(str::to_string),
        ),
        UiuxAction::DesignSystem => parts.extend(
            ["设计系统", "design system", "color", "typography", "component", "token", "state"]
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
    if let Some(parent) = path.parent().and_then(|value| value.file_name()).and_then(|value| value.to_str()) {
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
            let location = snippet
                .location
                .as_deref()
                .unwrap_or("未知位置");
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
    let normalized = location.to_lowercase().replace('\\', "/");
    if normalized.contains(UIUX_MARKDOWN_FILENAME)
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
