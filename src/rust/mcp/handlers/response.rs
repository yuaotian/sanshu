use anyhow::Result;
use rmcp::model::{ErrorData as McpError, Content};

use crate::mcp::types::{FileReferenceAttachment, McpResponse};
use crate::log_debug;

/// 解析 MCP 响应内容
///
/// 解析前端结构化响应格式，生成适当的 Content 对象
pub fn parse_mcp_response(response: &str) -> Result<Vec<Content>, McpError> {
    let trimmed = response.trim().trim_matches('"');
    if trimmed == "CANCELLED" || trimmed == "用户取消了操作" {
        log_debug!("[parse_mcp_response] 收到取消信号");
        return Ok(vec![Content::text("用户取消了操作，请询问用户下一步。".to_string())]);
    }

    match serde_json::from_str::<McpResponse>(response) {
        Ok(structured_response) => {
            log_debug!(
                "[parse_mcp_response] 结构化响应: selected_options={}, images={}, files={}, request_id={:?}, source={:?}",
                structured_response.selected_options.len(),
                structured_response.images.len(),
                structured_response.files.len(),
                structured_response.metadata.request_id.as_deref(),
                structured_response.metadata.source.as_deref()
            );
            parse_structured_response(structured_response)
        }
        Err(_) => {
            log_debug!("[parse_mcp_response] 非结构化响应，按纯文本处理: len={}", response.len());
            Ok(vec![Content::text(response.to_string())])
        }
    }
}

/// 解析新的结构化响应格式
///
/// 将前端响应拆分为语义区域返回给 agent：
/// 1. 图片作为独立 Content::image（AI 直接可视）
/// 2. 用户消息（干净的用户意图）
/// 3. 附加上下文（选项 + 文件引用，不含冗余图片元信息）
/// 4. 执行偏好（条件性提示词 ✔/❌ 状态）
fn parse_structured_response(response: McpResponse) -> Result<Vec<Content>, McpError> {
    let mut result = Vec::new();

    // 图片作为独立 Content 对象（agent 可直接展示）
    for image in &response.images {
        result.push(Content::image(image.data.clone(), image.media_type.clone()));
    }

    let combined_text = build_structured_context_text(&response);
    if !combined_text.is_empty() {
        result.push(Content::text(combined_text));
    }

    if result.is_empty() {
        result.push(Content::text("用户未提供任何内容".to_string()));
    }

    Ok(result)
}

/// 构建结构化上下文文本
///
/// 将响应数据组织为 agent 友好的分区格式，
/// 让 agent 能清晰区分用户意图、附件信息和执行约束
fn build_structured_context_text(response: &McpResponse) -> String {
    let mut sections = Vec::new();
    let mut user_message_parts = Vec::new();
    let mut preference_lines = Vec::new();

    if let Some(user_input) = response.user_input.as_ref() {
        let (user_message, preferences) = split_user_message_and_preferences(user_input);
        if !user_message.is_empty() {
            user_message_parts.push(user_message);
        }
        preference_lines.extend(preferences);
    }

    // 区域 1：用户消息
    if !user_message_parts.is_empty() {
        sections.push(user_message_parts.join("\n"));
    }

    // 区域 2：附加上下文（选项 + 文件引用）
    let mut context_lines = Vec::new();

    if !response.selected_options.is_empty() {
        let options_json: Vec<String> = response.selected_options.iter()
            .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "\"\"".to_string()))
            .collect();
        context_lines.push(format!("- 选项: [{}]", options_json.join(", ")));
    }

    let mut resource_index = 0usize;
    for img in &response.images {
        resource_index += 1;
        let name = img.filename.as_deref().unwrap_or("unnamed");
        context_lines.push(format!("- 资源{}: [image] {}", resource_index, name));
    }
    for file in &response.files {
        resource_index += 1;
        context_lines.push(format!("- 资源{}: {}", resource_index, format_file_reference_compact(file)));
    }

    if !context_lines.is_empty() {
        sections.push(format!("附加上下文：\n{}", context_lines.join("\n")));
    }

    if !preference_lines.is_empty() {
        let pref_text: Vec<String> = preference_lines.into_iter()
            .map(|line| format!("- {}", line))
            .collect();
        sections.push(format!("执行偏好：\n{}", pref_text.join("\n")));
    }

    sections.join("\n\n")
}

/// 将用户输入文本拆分为纯消息和偏好指令
///
/// ✔ / ❌ 开头的行被识别为条件性提示词状态，从用户消息中分离出来
fn split_user_message_and_preferences(user_input: &str) -> (String, Vec<String>) {
    let mut message_lines = Vec::new();
    let mut preference_lines = Vec::new();

    for line in user_input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !message_lines.is_empty() {
                message_lines.push(String::new());
            }
            continue;
        }

        if is_preference_line(trimmed) {
            preference_lines.push(trimmed.to_string());
        } else {
            message_lines.push(trimmed.to_string());
        }
    }

    while message_lines.last().is_some_and(|line| line.is_empty()) {
        message_lines.pop();
    }

    (message_lines.join("\n"), preference_lines)
}

fn is_preference_line(line: &str) -> bool {
    line.starts_with('✔') || line.starts_with('❌')
}

fn format_file_reference_compact(file: &FileReferenceAttachment) -> String {
    if file.r#type == "url" {
        let url = file.url.as_deref().unwrap_or_default();
        format!("[url] {}", url)
    } else {
        let path = file.path.as_deref().unwrap_or_default();
        let kind_tag = if file.kind.as_deref() == Some("directory") { "dir" } else { "file" };
        format!("[{}] {}", kind_tag, path)
    }
}
