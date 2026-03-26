use anyhow::Result;
use rmcp::model::{ErrorData as McpError, Content};

use crate::mcp::types::{FileReferenceAttachment, ImageAttachment, McpResponse, McpResponseContent};
use crate::log_debug;

/// 解析 MCP 响应内容
///
/// 支持新的结构化格式和旧格式的兼容性，并生成适当的 Content 对象
pub fn parse_mcp_response(response: &str) -> Result<Vec<Content>, McpError> {
    if response.trim() == "CANCELLED" || response.trim() == "用户取消了操作" {
        log_debug!("[parse_mcp_response] 收到取消信号");
        return Ok(vec![Content::text("用户取消了操作".to_string())]);
    }

    // 首先尝试解析为新的结构化格式
    if let Ok(structured_response) = serde_json::from_str::<McpResponse>(response) {
        log_debug!(
            "[parse_mcp_response] 结构化响应: selected_options={}, images={}, files={}, request_id={:?}, source={:?}",
            structured_response.selected_options.len(),
            structured_response.images.len(),
            structured_response.files.len(),
            structured_response.metadata.request_id.as_deref(),
            structured_response.metadata.source.as_deref()
        );
        return parse_structured_response(structured_response);
    }

    // 回退到旧格式兼容性解析
    match serde_json::from_str::<Vec<McpResponseContent>>(response) {
        Ok(content_array) => {
            log_debug!("[parse_mcp_response] 旧格式响应数组: items={}", content_array.len());
            let mut result = Vec::new();
            let mut image_count = 0;

            let mut user_text_parts = Vec::new();
            let mut image_info_parts = Vec::new();

            for content in content_array {
                match content.content_type.as_str() {
                    "text" => {
                        if let Some(text) = content.text {
                            user_text_parts.push(text);
                        }
                    }
                    "image" => {
                        if let Some(source) = content.source {
                            if source.source_type == "base64" {
                                image_count += 1;
                                result.push(Content::image(source.data.clone(), source.media_type.clone()));

                                let estimated_size = (source.data.len() * 3) / 4;
                                let size_str = format_byte_size(estimated_size);
                                image_info_parts.push(format!(
                                    "=== 图片 {} ===\n类型: {}\n大小: {}",
                                    image_count, source.media_type, size_str
                                ));
                            }
                        }
                    }
                    _ => {
                        if let Some(text) = content.text {
                            user_text_parts.push(text);
                        }
                    }
                }
            }

            let mut all_text_parts = Vec::new();
            if !user_text_parts.is_empty() {
                all_text_parts.extend(user_text_parts);
            }
            if !image_info_parts.is_empty() {
                all_text_parts.extend(image_info_parts);
            }
            if image_count > 0 {
                all_text_parts.push(format!(
                    "💡 注意：用户提供了 {} 张图片。如果 AI 助手无法显示图片，图片数据已包含在上述 Base64 信息中。",
                    image_count
                ));
            }

            if !all_text_parts.is_empty() {
                result.push(Content::text(all_text_parts.join("\n\n")));
            }
            if result.is_empty() {
                result.push(Content::text("用户未提供任何内容".to_string()));
            }

            log_debug!(
                "[parse_mcp_response] 旧格式解析完成: images={}, content_items={}",
                image_count, result.len()
            );
            Ok(result)
        }
        Err(_) => {
            log_debug!("[parse_mcp_response] 非JSON响应，按纯文本处理: len={}", response.len());
            Ok(vec![Content::text(response.to_string())])
        }
    }
}

/// 解析新的结构化响应格式
///
/// 将前端响应拆分为三个语义区域返回给 agent：
/// 1. 用户消息（干净的用户意图）
/// 2. 附加上下文（选项、文件引用、图片元信息）
/// 3. 执行偏好（条件性提示词 ✔/❌ 状态）
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

    // 区域 2：附加上下文（选项 + 文件引用 + 图片元信息）
    let mut context_lines = Vec::new();

    if !response.selected_options.is_empty() {
        let options_json: Vec<String> = response.selected_options.iter()
            .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "\"\"".to_string()))
            .collect();
        context_lines.push(format!("- 选项: [{}]", options_json.join(", ")));
    }

    if !response.files.is_empty() {
        let references: Vec<String> = response.files.iter().enumerate()
            .map(|(i, file)| format!("- 资源{}: {}", i + 1, format_file_reference(file)))
            .collect();
        context_lines.extend(references);
    }

    if !response.images.is_empty() {
        let images: Vec<String> = response.images.iter().enumerate()
            .map(|(i, img)| format!("- 图片{}: {}", i + 1, format_image_attachment(img)))
            .collect();
        context_lines.extend(images);
    }

    if !context_lines.is_empty() {
        sections.push(format!("附加上下文：\n{}", context_lines.join("\n")));
    }

    // 区域 3：执行偏好（条件性提示词状态）
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

fn format_file_reference(file: &FileReferenceAttachment) -> String {
    let reference_kind = if file.r#type == "url" {
        "url"
    } else if file.kind.as_deref() == Some("directory") {
        "directory"
    } else {
        "file"
    };

    let location = if file.r#type == "url" {
        file.url.as_deref().unwrap_or_default()
    } else {
        file.path.as_deref().unwrap_or_default()
    };

    let mut fields = vec![
        format!("type: {}", serde_json::to_string(reference_kind).unwrap_or_default()),
        format!("name: {}", serde_json::to_string(&file.name).unwrap_or_default()),
    ];

    if file.r#type == "url" {
        fields.push(format!("url: {}", serde_json::to_string(location).unwrap_or_default()));
    } else {
        fields.push(format!("path: {}", serde_json::to_string(location).unwrap_or_default()));
    }

    if let Some(mime_type) = file.mime_type.as_ref() {
        fields.push(format!("mime_type: {}", serde_json::to_string(mime_type).unwrap_or_default()));
    }

    format!("{{ {} }}", fields.join(", "))
}

fn format_image_attachment(image: &ImageAttachment) -> String {
    let estimated_size = (image.data.len() * 3) / 4;
    let size_str = format_byte_size(estimated_size);

    let mut fields = vec![
        format!("media_type: {}", serde_json::to_string(&image.media_type).unwrap_or_default()),
        format!("size: {}", serde_json::to_string(&size_str).unwrap_or_default()),
    ];

    if let Some(filename) = image.filename.as_ref() {
        fields.push(format!("filename: {}", serde_json::to_string(filename).unwrap_or_default()));
    }

    format!("{{ {} }}", fields.join(", "))
}

fn format_byte_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
