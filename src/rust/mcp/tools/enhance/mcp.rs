// MCP 工具入口
// 将提示词增强功能注册为 MCP 工具，供 AI 编辑器直接调用

use std::borrow::Cow;
use std::sync::Arc;
use rmcp::model::{Tool, CallToolResult, Content, ErrorData as McpError};
use serde::{Deserialize, Serialize};

use super::types::*;
use super::core::PromptEnhancer;
use super::history::ChatHistoryManager;
use crate::log_important;

/// MCP 增强工具请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhanceMcpRequest {
    /// 要增强的提示词
    pub prompt: String,
    /// 项目根路径（可选）
    #[serde(default)]
    pub project_root_path: Option<String>,
    /// 是否包含对话历史（可选）
    #[serde(default)]
    pub include_history: Option<bool>,
}

/// 提示词增强 MCP 工具
pub struct EnhanceTool;

impl EnhanceTool {
    /// 获取工具定义
    pub fn get_tool_definition() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "要增强的原始提示词。工具将分析并改写为更清晰、具体、无歧义的专业提示词。"
                },
                "project_root_path": {
                    "type": "string",
                    "description": "项目根目录的绝对路径（可选）。如果提供，将利用项目的代码上下文来提升增强质量。"
                },
                "include_history": {
                    "type": "boolean",
                    "description": "是否包含对话历史（可选，默认 true）。"
                }
            },
            "required": ["prompt"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            Tool {
                name: Cow::Borrowed("enhance"),
                description: Some(Cow::Borrowed(
                    "提示词增强工具。将口语化的提示词转换为结构化、精确的专业提示词。利用项目代码上下文和对话历史提升增强质量。"
                )),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            }
        } else {
            panic!("Invalid schema")
        }
    }

    /// 执行增强
    pub async fn enhance(request: EnhanceMcpRequest) -> Result<CallToolResult, McpError> {
        log_important!(info, "MCP enhance 工具被调用: prompt_len={}", request.prompt.len());

        // 创建增强器
        let enhancer = match PromptEnhancer::from_acemcp_config().await {
            Ok(mut e) => {
                if let Some(ref path) = request.project_root_path {
                    e = e.with_project_root(path);
                }
                e
            }
            Err(e) => {
                return Err(McpError::internal_error(
                    format!("初始化增强器失败: {}", e),
                    None
                ));
            }
        };

        let project_root_path = request.project_root_path.clone();
        let include_history = request.include_history.unwrap_or(true);

        let enhance_request = EnhanceRequest {
            prompt: request.prompt.clone(),
            project_root_path: project_root_path.clone(),
            current_file_path: None,
            include_history,
        };

        match enhancer.enhance(enhance_request).await {
            Ok(response) => {
                if response.success {
                    // 记录对话历史（仅在提供项目路径时）
                    if let Some(ref path) = project_root_path {
                        if let Ok(manager) = ChatHistoryManager::new(path) {
                            let _ = manager.add_entry(
                                &request.prompt,
                                &response.enhanced_prompt,
                                "mcp"
                            );
                        }
                    }
                    // 成功：返回增强后的提示词
                    let result_text = format!(
                        "## 增强后的提示词\n\n{}\n\n---\n*使用了 {} 个代码上下文块，{} 条对话历史*",
                        response.enhanced_prompt,
                        response.blob_count,
                        response.history_count
                    );
                    Ok(CallToolResult::success(vec![Content::text(result_text)]))
                } else {
                    // 失败：返回错误信息
                    let error_text = format!(
                        "增强失败: {}",
                        response.error.unwrap_or_else(|| "未知错误".to_string())
                    );
                    Ok(CallToolResult::success(vec![Content::text(error_text)]))
                }
            }
            Err(e) => {
                Err(McpError::internal_error(
                    format!("增强执行失败: {}", e),
                    None
                ))
            }
        }
    }
}
