// 提示词增强核心逻辑
// 调用 Augment chat-stream API 实现流式提示词增强

use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use anyhow::Result;
use reqwest::{Client, header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue}};
use serde_json::json;
use regex::Regex;
use futures_util::StreamExt;

use super::types::*;
use super::history::ChatHistoryManager;
use crate::mcp::tools::acemcp::mcp::ProjectsFile;
use crate::{log_debug, log_important};

/// 增强系统提示词模板
const ENHANCE_SYSTEM_PROMPT: &str = r#"⚠️ NO TOOLS ALLOWED ⚠️

Here is an instruction that I'd like to give you, but it needs to be improved. Rewrite and enhance this instruction to make it clearer, more specific, less ambiguous, and correct any mistakes. Do not use any tools: reply immediately with your answer, even if you're not sure. Consider the context of our conversation history when enhancing the prompt. If there is code in triple backticks (```) consider whether it is a code sample and should remain unchanged.Reply with the following format:

### BEGIN RESPONSE ###
Here is an enhanced version of the original instruction that is more specific and clear:
<augment-enhanced-prompt>enhanced prompt goes here</augment-enhanced-prompt>

### END RESPONSE ###

Here is my original instruction:

"#;

/// 提示词增强器
pub struct PromptEnhancer {
    /// Augment API 基础 URL
    base_url: String,
    /// API Token
    token: String,
    /// HTTP 客户端
    client: Client,
    /// 项目根路径
    project_root: Option<String>,
}

impl PromptEnhancer {
    /// 创建增强器实例
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;

        Ok(Self {
            base_url: normalize_base_url(base_url),
            token: token.to_string(),
            client,
            project_root: None,
        })
    }

    /// 设置项目根路径
    pub fn with_project_root(mut self, path: &str) -> Self {
        self.project_root = Some(path.to_string());
        self
    }

    /// 从 acemcp 配置创建增强器
    pub async fn from_acemcp_config() -> Result<Self> {
        use crate::mcp::tools::acemcp::AcemcpTool;
        
        let config = AcemcpTool::get_acemcp_config().await?;
        let base_url = config.base_url
            .ok_or_else(|| anyhow::anyhow!("未配置 Acemcp base_url"))?;
        let token = config.token
            .ok_or_else(|| anyhow::anyhow!("未配置 Acemcp token"))?;

        Self::new(&base_url, &token)
    }

    /// 加载项目的 blob_names
    fn load_blob_names(&self) -> Vec<String> {
        let project_root = match &self.project_root {
            Some(path) => path.clone(),
            None => return Vec::new(),
        };

        let projects_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".sanshu")
            .join("projects.json");

        if !projects_path.exists() {
            log_debug!("projects.json 不存在，跳过 blob 加载");
            return Vec::new();
        }

        let content = match fs::read_to_string(&projects_path) {
            Ok(c) => c,
            Err(e) => {
                log_debug!("读取 projects.json 失败: {}", e);
                return Vec::new();
            }
        };

        let projects: ProjectsFile = match serde_json::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
                log_debug!("解析 projects.json 失败: {}", e);
                return Vec::new();
            }
        };

        // 规范化项目路径
        let normalized_root = PathBuf::from(&project_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&project_root))
            .to_string_lossy()
            .replace('\\', "/");

        projects.0.get(&normalized_root).cloned().unwrap_or_default()
    }

    /// 加载对话历史
    fn load_chat_history(&self, count: usize) -> Vec<ChatHistoryEntry> {
        let project_root = match &self.project_root {
            Some(path) => path.clone(),
            None => return Vec::new(),
        };

        match ChatHistoryManager::new(&project_root) {
            Ok(manager) => manager.to_api_format(count),
            Err(e) => {
                log_debug!("加载对话历史失败: {}", e);
                Vec::new()
            }
        }
    }

    /// 构建 chat-stream 请求体
    fn build_request_payload(&self, prompt: &str, current_file: Option<&str>, include_history: bool) -> serde_json::Value {
        let blob_names = self.load_blob_names();
        let chat_history = if include_history {
            self.load_chat_history(5) // 最多5条历史
        } else {
            Vec::new()
        };

        log_important!(info, "构建增强请求: blob_count={}, history_count={}", blob_names.len(), chat_history.len());

        // 构建完整消息
        let full_message = format!("{}{}", ENHANCE_SYSTEM_PROMPT, prompt);

        json!({
            "model": "claude-sonnet-4-5",
            "path": current_file.unwrap_or(""),
            "prefix": null,
            "selected_code": null,
            "suffix": null,
            "message": full_message,
            "chat_history": chat_history,
            "lang": "",
            "blobs": {
                "checkpoint_id": null,
                "added_blobs": blob_names,
                "deleted_blobs": []
            },
            "user_guided_blobs": [],
            "context_code_exchange_request_id": "new",
            "external_source_ids": [],
            "disable_auto_external_sources": null,
            "user_guidelines": "",
            "workspace_guidelines": "",
            "feature_detection_flags": {
                "support_tool_use_start": true,
                "support_parallel_tool_use": true
            },
            "tool_definitions": [],
            "nodes": [
                {
                    "id": 1,
                    "type": 0,
                    "text_node": {
                        "content": full_message
                    }
                }
            ],
            "mode": "CHAT",
            "agent_memories": null,
            "persona_type": 1,
            "rules": [],
            "silent": true,
            "third_party_override": null,
            "conversation_id": uuid::Uuid::new_v4().to_string(),
            "canvas_id": null
        })
    }

    /// 从响应文本中提取增强后的提示词
    pub fn extract_enhanced_prompt(text: &str) -> Option<String> {
        // 匹配 <augment-enhanced-prompt>...</augment-enhanced-prompt>
        let re = Regex::new(r"<augment-enhanced-prompt>([\s\S]*?)</augment-enhanced-prompt>").ok()?;
        re.captures(text)?
            .get(1)
            .map(|m| m.as_str().trim().to_string())
    }

    /// 同步增强（等待完成后返回）
    pub async fn enhance(&self, request: EnhanceRequest) -> Result<EnhanceResponse> {
        let payload = self.build_request_payload(
            &request.prompt,
            request.current_file_path.as_deref(),
            request.include_history,
        );

        let blob_count = payload.get("blobs")
            .and_then(|b| b.get("added_blobs"))
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let history_count = payload.get("chat_history")
            .and_then(|h| h.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let url = format!("{}/chat-stream", self.base_url);
        log_important!(info, "发送增强请求: url={}", url);

        let response = self.client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Ok(EnhanceResponse {
                enhanced_prompt: String::new(),
                original_prompt: request.prompt,
                success: false,
                error: Some(format!("HTTP {} - {}", status, body)),
                blob_count,
                history_count,
            });
        }

        // 处理 SSE 流式响应
        let mut accumulated_text = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    // SSE 响应每行是一个 JSON 对象
                    for line in text.lines() {
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(text_chunk) = json.get("text").and_then(|t| t.as_str()) {
                                accumulated_text.push_str(text_chunk);
                            }
                        }
                    }
                }
                Err(e) => {
                    log_debug!("读取流式响应失败: {}", e);
                }
            }
        }

        // 提取增强后的提示词
        let enhanced_prompt = Self::extract_enhanced_prompt(&accumulated_text)
            .unwrap_or_default();

        let success = !enhanced_prompt.is_empty();

        Ok(EnhanceResponse {
            enhanced_prompt,
            original_prompt: request.prompt,
            success,
            error: if success { None } else { Some("未能从响应中提取增强结果".to_string()) },
            blob_count,
            history_count,
        })
    }

    /// 流式增强（通过回调函数推送进度）
    pub async fn enhance_stream<F>(&self, request: EnhanceRequest, mut on_event: F) -> Result<EnhanceResponse>
    where
        F: FnMut(EnhanceStreamEvent) + Send,
    {
        let payload = self.build_request_payload(
            &request.prompt,
            request.current_file_path.as_deref(),
            request.include_history,
        );

        let blob_count = payload.get("blobs")
            .and_then(|b| b.get("added_blobs"))
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let history_count = payload.get("chat_history")
            .and_then(|h| h.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        let url = format!("{}/chat-stream", self.base_url);
        log_important!(info, "发送流式增强请求: url={}", url);

        let response = self.client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let error_msg = format!("HTTP {} - {}", status, body);
            on_event(EnhanceStreamEvent::error(&error_msg));
            return Ok(EnhanceResponse {
                enhanced_prompt: String::new(),
                original_prompt: request.prompt,
                success: false,
                error: Some(error_msg),
                blob_count,
                history_count,
            });
        }

        // 处理 SSE 流式响应
        let mut accumulated_text = String::new();
        let mut stream = response.bytes_stream();
        let mut chunk_count = 0u32;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(text_chunk) = json.get("text").and_then(|t| t.as_str()) {
                                if !text_chunk.is_empty() {
                                    accumulated_text.push_str(text_chunk);
                                    chunk_count += 1;
                                    
                                    // 估算进度（基于常见响应长度）
                                    let progress = std::cmp::min(90, (chunk_count * 2) as u8);
                                    
                                    on_event(EnhanceStreamEvent::chunk(
                                        text_chunk,
                                        &accumulated_text,
                                        progress,
                                    ));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log_debug!("读取流式响应失败: {}", e);
                }
            }
        }

        // 提取增强后的提示词
        let enhanced_prompt = Self::extract_enhanced_prompt(&accumulated_text)
            .unwrap_or_default();

        let success = !enhanced_prompt.is_empty();

        if success {
            on_event(EnhanceStreamEvent::complete(&enhanced_prompt, &accumulated_text));
        } else {
            on_event(EnhanceStreamEvent::error("未能从响应中提取增强结果"));
        }

        Ok(EnhanceResponse {
            enhanced_prompt,
            original_prompt: request.prompt,
            success,
            error: if success { None } else { Some("未能从响应中提取增强结果".to_string()) },
            blob_count,
            history_count,
        })
    }
}

/// 规范化 URL
fn normalize_base_url(input: &str) -> String {
    let mut url = input.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        url = format!("https://{}", url);
    }
    while url.ends_with('/') {
        url.pop();
    }
    url
}
