// Tavily AI 搜索 MCP 工具实现
// 支持 Search（实时搜索）和 Extract（内容提取）双端点

use anyhow::Result;
use rmcp::model::{ErrorData as McpError, Tool, CallToolResult, Content};
use reqwest::Client;
use serde_json::json;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use super::types::*;
use crate::{log_debug, log_important};

/// Tavily AI 搜索工具
pub struct TavilyTool;

impl TavilyTool {
    /// 执行 Tavily 工具调用（根据 action 分发到 search 或 extract）
    pub async fn execute(request: TavilyRequest) -> Result<CallToolResult, McpError> {
        let action = request.action.to_lowercase();
        log_important!(info,
            "Tavily 请求: action={}, query={:?}",
            action,
            request.query.as_deref().map(|s| if s.len() > 100 { format!("{}...", &s[..100]) } else { s.to_string() })
        );

        // 获取配置
        let config = Self::get_config()
            .await
            .map_err(|e| McpError::internal_error(format!("获取 Tavily 配置失败: {}", e), None))?;

        // 验证 API Key
        let api_key = config.api_key.as_ref().ok_or_else(|| {
            McpError::internal_error(
                "Tavily API Key 未配置。请在设置中配置 Tavily API Key。免费计划每月提供 1000 个 API 信用点。".to_string(),
                None,
            )
        })?;

        match action.as_str() {
            "search" => Self::search(&config, api_key, &request).await,
            "extract" => Self::extract(&config, api_key, &request).await,
            _ => Err(McpError::invalid_params(
                format!("未知的 action: {}。支持 'search'（默认）或 'extract'", action),
                None,
            )),
        }
    }

    /// 搜索端点
    async fn search(config: &TavilyConfig, api_key: &str, request: &TavilyRequest) -> Result<CallToolResult, McpError> {
        let query = request.query.as_deref().ok_or_else(|| {
            McpError::invalid_params("search 操作需要 query 参数".to_string(), None)
        })?;

        if query.trim().is_empty() {
            return Err(McpError::invalid_params("query 不能为空".to_string(), None));
        }

        let client = Self::create_client()?;
        let url = format!("{}/search", config.base_url);

        // 构建请求体
        let mut body = json!({
            "query": query,
            "include_usage": true,
        });

        // 填充可选参数
        if let Some(ref depth) = request.search_depth {
            body["search_depth"] = json!(depth);
        }
        if let Some(max) = request.max_results {
            body["max_results"] = json!(max.min(20));
        }
        if let Some(ref topic) = request.topic {
            body["topic"] = json!(topic);
        }
        if let Some(ref range) = request.time_range {
            body["time_range"] = json!(range);
        }
        if let Some(ref answer) = request.include_answer {
            body["include_answer"] = answer.clone();
        }
        if let Some(ref raw) = request.include_raw_content {
            body["include_raw_content"] = raw.clone();
        }
        if let Some(ref domains) = request.include_domains {
            if !domains.is_empty() {
                body["include_domains"] = json!(domains);
            }
        }
        if let Some(ref domains) = request.exclude_domains {
            if !domains.is_empty() {
                body["exclude_domains"] = json!(domains);
            }
        }

        log_debug!("Tavily Search 请求 URL: {}", url);

        // 发送请求
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                let msg = if e.is_timeout() {
                    "Tavily 搜索请求超时（30s）".to_string()
                } else if e.is_connect() {
                    "无法连接到 Tavily API，请检查网络连接".to_string()
                } else {
                    format!("Tavily 搜索请求失败: {}", e)
                };
                McpError::internal_error(msg, None)
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "无法读取错误信息".to_string());
            let msg = Self::format_http_error(status.as_u16(), &error_text);
            return Ok(CallToolResult {
                content: vec![Content::text(msg)],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            });
        }

        // 解析响应
        let response_text = response.text().await
            .map_err(|e| McpError::internal_error(format!("读取响应失败: {}", e), None))?;

        let search_response: TavilySearchResponse = serde_json::from_str(&response_text)
            .map_err(|e| McpError::internal_error(format!("解析搜索响应失败: {}", e), None))?;

        // 格式化输出
        let formatted = Self::format_search_result(&search_response);
        log_important!(info,
            "Tavily Search 完成: results={}, response_time={:?}ms, request_id={:?}",
            search_response.results.len(),
            search_response.response_time,
            search_response.request_id
        );

        Ok(CallToolResult {
            content: vec![Content::text(formatted)],
            is_error: Some(false),
            meta: None,
            structured_content: None,
        })
    }

    /// 内容提取端点
    async fn extract(config: &TavilyConfig, api_key: &str, request: &TavilyRequest) -> Result<CallToolResult, McpError> {
        let urls = request.urls.as_ref().ok_or_else(|| {
            McpError::invalid_params("extract 操作需要 urls 参数".to_string(), None)
        })?;

        // 将 urls 统一为数组格式
        let urls_array = match urls {
            serde_json::Value::String(s) => vec![s.clone()],
            serde_json::Value::Array(arr) => {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            }
            _ => {
                return Err(McpError::invalid_params(
                    "urls 参数必须是字符串或字符串数组".to_string(),
                    None,
                ));
            }
        };

        if urls_array.is_empty() {
            return Err(McpError::invalid_params("urls 不能为空".to_string(), None));
        }

        let client = Self::create_client()?;
        let url = format!("{}/extract", config.base_url);

        // 构建请求体
        let mut body = json!({
            "urls": urls_array,
            "include_usage": true,
        });

        if let Some(ref depth) = request.extract_depth {
            body["extract_depth"] = json!(depth);
        }
        // extract 也支持 query 用于重排序
        if let Some(ref query) = request.query {
            if !query.trim().is_empty() {
                body["query"] = json!(query);
            }
        }

        log_debug!("Tavily Extract 请求 URL: {}", url);

        // 发送请求
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::internal_error(format!("Tavily 提取请求失败: {}", e), None))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "无法读取错误信息".to_string());
            let msg = Self::format_http_error(status.as_u16(), &error_text);
            return Ok(CallToolResult {
                content: vec![Content::text(msg)],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            });
        }

        let response_text = response.text().await
            .map_err(|e| McpError::internal_error(format!("读取响应失败: {}", e), None))?;

        let extract_response: TavilyExtractResponse = serde_json::from_str(&response_text)
            .map_err(|e| McpError::internal_error(format!("解析提取响应失败: {}", e), None))?;

        let formatted = Self::format_extract_result(&extract_response);
        log_important!(info,
            "Tavily Extract 完成: results={}, failed={}, response_time={:?}ms",
            extract_response.results.len(),
            extract_response.failed_results.len(),
            extract_response.response_time
        );

        Ok(CallToolResult {
            content: vec![Content::text(formatted)],
            is_error: Some(false),
            meta: None,
            structured_content: None,
        })
    }

    /// 获取工具定义
    pub fn get_tool_definition() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索查询关键词或问题（search 时必填，extract 时可选用于重排序）"
                },
                "action": {
                    "type": "string",
                    "enum": ["search", "extract"],
                    "description": "操作类型：search（AI搜索，默认）或 extract（内容提取）"
                },
                "search_depth": {
                    "type": "string",
                    "enum": ["basic", "advanced"],
                    "description": "搜索深度：basic（1信用点）或 advanced（2信用点，更全面）"
                },
                "max_results": {
                    "type": "integer",
                    "description": "最大搜索结果数量（0-20，默认5）"
                },
                "topic": {
                    "type": "string",
                    "enum": ["general", "news", "finance"],
                    "description": "搜索类别：general（通用）、news（新闻实时）、finance（财经）"
                },
                "time_range": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year"],
                    "description": "时间范围过滤"
                },
                "include_answer": {
                    "description": "是否包含 AI 生成的回答：false（默认）、\"basic\" 或 \"advanced\""
                },
                "include_raw_content": {
                    "description": "是否包含清理后的原始内容：false（默认）、\"markdown\" 或 \"text\""
                },
                "include_domains": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "域名白名单（仅包含这些域名的结果）"
                },
                "exclude_domains": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "域名黑名单（排除这些域名的结果）"
                },
                "urls": {
                    "description": "提取目标 URL（extract 时必填，支持单个字符串或字符串数组，最多20个）"
                },
                "extract_depth": {
                    "type": "string",
                    "enum": ["basic", "advanced"],
                    "description": "提取深度：basic 或 advanced"
                }
            },
            "required": ["query"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            Tool {
                name: Cow::Borrowed("tavily"),
                description: Some(Cow::Borrowed(
                    "AI 搜索与内容提取工具。search：实时搜索互联网获取最新信息，支持 AI 回答生成；extract：从指定 URL 提取结构化内容。免费额度每月1000信用点。"
                )),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("Tavily AI 搜索".to_string()),
            }
        } else {
            panic!("Schema creation failed");
        }
    }

    /// 获取配置
    async fn get_config() -> Result<TavilyConfig> {
        let config = crate::config::load_standalone_config()
            .map_err(|e| anyhow::anyhow!("读取配置文件失败: {}", e))?;

        Ok(TavilyConfig {
            api_key: config.mcp_config.tavily_api_key,
            base_url: "https://api.tavily.com".to_string(),
        })
    }

    /// 创建 HTTP 客户端
    fn create_client() -> Result<Client, McpError> {
        Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| McpError::internal_error(format!("创建 HTTP 客户端失败: {}", e), None))
    }

    /// 格式化搜索结果为可读文本
    fn format_search_result(response: &TavilySearchResponse) -> String {
        let mut output = String::new();

        // AI 回答
        if let Some(ref answer) = response.answer {
            if !answer.is_empty() {
                output.push_str("## AI 回答\n\n");
                output.push_str(answer);
                output.push_str("\n\n---\n\n");
            }
        }

        // 搜索结果
        if response.results.is_empty() {
            output.push_str("未找到相关搜索结果。\n");
        } else {
            output.push_str(&format!("## 搜索结果（共 {} 条）\n\n", response.results.len()));
            for (i, result) in response.results.iter().enumerate() {
                let title = result.title.as_deref().unwrap_or("无标题");
                output.push_str(&format!("### {}. {}\n", i + 1, title));
                output.push_str(&format!("**URL:** {}\n", result.url));
                if let Some(score) = result.score {
                    output.push_str(&format!("**相关度:** {:.2}\n", score));
                }
                if let Some(ref content) = result.content {
                    if !content.is_empty() {
                        output.push_str(&format!("\n{}\n", content));
                    }
                }
                if let Some(ref raw) = result.raw_content {
                    if !raw.is_empty() {
                        // 截断过长的原始内容
                        let truncated = if raw.len() > 2000 {
                            format!("{}...\n\n（内容已截断，原始长度: {} 字符）", &raw[..2000], raw.len())
                        } else {
                            raw.clone()
                        };
                        output.push_str(&format!("\n**原始内容:**\n{}\n", truncated));
                    }
                }
                output.push('\n');
            }
        }

        // 元信息
        if let Some(time) = response.response_time {
            output.push_str(&format!("\n_响应时间: {:.2}s_", time));
        }

        output
    }

    /// 格式化提取结果为可读文本
    fn format_extract_result(response: &TavilyExtractResponse) -> String {
        let mut output = String::new();

        if response.results.is_empty() && response.failed_results.is_empty() {
            output.push_str("未获取到任何提取结果。\n");
            return output;
        }

        // 成功的提取结果
        if !response.results.is_empty() {
            output.push_str(&format!("## 提取结果（共 {} 条）\n\n", response.results.len()));
            for (i, result) in response.results.iter().enumerate() {
                output.push_str(&format!("### {}. {}\n", i + 1, result.url));
                if let Some(ref content) = result.raw_content {
                    if !content.is_empty() {
                        // 截断过长内容
                        let truncated = if content.len() > 5000 {
                            format!("{}...\n\n（内容已截断，原始长度: {} 字符）", &content[..5000], content.len())
                        } else {
                            content.clone()
                        };
                        output.push_str(&format!("\n{}\n", truncated));
                    }
                }
                output.push('\n');
            }
        }

        // 失败的提取结果
        if !response.failed_results.is_empty() {
            output.push_str(&format!("## 提取失败（{} 条）\n\n", response.failed_results.len()));
            for result in &response.failed_results {
                let error = result.error.as_deref().unwrap_or("未知错误");
                output.push_str(&format!("- {} — {}\n", result.url, error));
            }
        }

        // 元信息
        if let Some(time) = response.response_time {
            output.push_str(&format!("\n_响应时间: {:.2}s_", time));
        }

        output
    }

    /// 格式化 HTTP 错误信息
    fn format_http_error(status: u16, body: &str) -> String {
        match status {
            401 => "❌ Tavily API Key 无效或已过期。请在设置中检查并更新 API Key。".to_string(),
            429 => "❌ Tavily API 请求频率超限。免费计划限制每分钟 100 次请求。请稍后重试。".to_string(),
            402 => "❌ Tavily API 信用点已耗尽。免费计划每月 1000 信用点。".to_string(),
            _ => {
                let preview = if body.len() > 300 { &body[..300] } else { body };
                format!("❌ Tavily API 错误 (HTTP {}): {}", status, preview)
            }
        }
    }
}
