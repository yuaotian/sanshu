use anyhow::Result;
use rmcp::model::{Content, ErrorData as McpError};
use serde_json::Value;

use crate::log_debug;
use crate::mcp::types::{McpResponse, McpResponseContent, ResponseContextBlock};
use crate::mcp::utils::is_zhi_custom_choice;

pub struct ParsedMcpResponse {
    pub content: Vec<Content>,
    pub structured_content: Option<Value>,
}

/// 解析 MCP 响应内容
///
/// 支持新的结构化格式和旧格式的兼容性，并生成适当的 Content 对象
pub fn parse_mcp_response(response: &str) -> Result<Vec<Content>, McpError> {
    Ok(parse_mcp_response_with_structured(response)?.content)
}

/// 解析 MCP 响应内容，并在新结构化响应中保留 structured_content。
pub fn parse_mcp_response_with_structured(response: &str) -> Result<ParsedMcpResponse, McpError> {
    if response.trim() == "CANCELLED" || response.trim() == "用户取消了操作" {
        log_debug!("[parse_mcp_response] 收到取消信号");
        return Ok(ParsedMcpResponse {
            content: vec![Content::text("用户取消了操作".to_string())],
            structured_content: None,
        });
    }

    // 首先尝试解析为新的结构化格式
    if let Ok(structured_response) = serde_json::from_str::<McpResponse>(response) {
        log_debug!(
            "[parse_mcp_response] 结构化响应: selected_options={}, images={}, request_id={:?}, source={:?}",
            structured_response.selected_options.len(),
            structured_response.images.len(),
            structured_response.metadata.request_id.as_deref(),
            structured_response.metadata.source.as_deref()
        );
        return parse_structured_response(structured_response);
    }

    // 回退到旧格式兼容性解析
    match serde_json::from_str::<Vec<McpResponseContent>>(response) {
        Ok(content_array) => {
            log_debug!(
                "[parse_mcp_response] 旧格式响应数组: items={}",
                content_array.len()
            );
            let mut result = Vec::new();
            let mut image_count = 0;

            // 分别收集用户文本和图片信息
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

                                // 先添加图片到结果中（图片在前）
                                result.push(Content::image(
                                    source.data.clone(),
                                    source.media_type.clone(),
                                ));

                                // 添加图片信息到图片信息部分
                                let base64_len = source.data.len();
                                let preview = if base64_len > 50 {
                                    format!("{}...", &source.data[..50])
                                } else {
                                    source.data.clone()
                                };

                                // 计算图片大小（base64解码后的大小）
                                let estimated_size = (base64_len * 3) / 4; // base64编码后大约增加33%
                                let size_str = if estimated_size < 1024 {
                                    format!("{} B", estimated_size)
                                } else if estimated_size < 1024 * 1024 {
                                    format!("{:.1} KB", estimated_size as f64 / 1024.0)
                                } else {
                                    format!("{:.1} MB", estimated_size as f64 / (1024.0 * 1024.0))
                                };

                                let image_info = format!(
                                    "=== 图片 {} ===\n类型: {}\n大小: {}\nBase64 预览: {}\n完整 Base64 长度: {} 字符",
                                    image_count, source.media_type, size_str, preview, base64_len
                                );
                                image_info_parts.push(image_info);
                            }
                        }
                    }
                    _ => {
                        // 未知类型，作为文本处理
                        if let Some(text) = content.text {
                            user_text_parts.push(text);
                        }
                    }
                }
            }

            // 构建文本内容：用户文本 + 图片信息 + 注意事项
            let mut all_text_parts = Vec::new();

            // 1. 用户输入的文本
            if !user_text_parts.is_empty() {
                all_text_parts.extend(user_text_parts);
            }

            // 2. 图片详细信息
            if !image_info_parts.is_empty() {
                all_text_parts.extend(image_info_parts);
            }

            // 3. 兼容性说明
            if image_count > 0 {
                all_text_parts.push(format!(
                    "💡 注意：用户提供了 {} 张图片。如果 AI 助手无法显示图片，图片数据已包含在上述 Base64 信息中。",
                    image_count
                ));
            }

            // 将所有文本内容合并并添加到结果末尾（图片后面）
            if !all_text_parts.is_empty() {
                let combined_text = all_text_parts.join("\n\n");
                result.push(Content::text(combined_text));
            }

            if result.is_empty() {
                result.push(Content::text("用户未提供任何内容".to_string()));
            }

            log_debug!(
                "[parse_mcp_response] 旧格式解析完成: images={}, content_items={}",
                image_count,
                result.len()
            );
            Ok(ParsedMcpResponse {
                content: result,
                structured_content: None,
            })
        }
        Err(_) => {
            // 如果不是JSON格式，作为纯文本处理
            log_debug!(
                "[parse_mcp_response] 非JSON响应，按纯文本处理: len={}",
                response.len()
            );
            Ok(ParsedMcpResponse {
                content: vec![Content::text(response.to_string())],
                structured_content: None,
            })
        }
    }
}

/// 解析新的结构化响应格式
fn parse_structured_response(response: McpResponse) -> Result<ParsedMcpResponse, McpError> {
    let mut result = Vec::new();
    let mut text_parts = Vec::new();

    let custom_selected = response
        .selected_options
        .iter()
        .any(|option| is_zhi_custom_choice(option));

    // 1. 处理选择的选项。自定义选项需要明确表达“以补充说明为准”，降低模型误读风险。
    if custom_selected {
        text_parts
            .push("用户选择了自定义要求：不采用以上预设选项，以补充说明为最终要求。".to_string());
        let non_custom_options: Vec<&str> = response
            .selected_options
            .iter()
            .filter(|option| !is_zhi_custom_choice(option))
            .map(String::as_str)
            .collect();
        if !non_custom_options.is_empty() {
            text_parts.push(format!(
                "同时选中的其他选项仅供参考，不应优先于自定义要求: {}",
                non_custom_options.join(", ")
            ));
        }
    } else if !response.selected_options.is_empty() {
        text_parts.push(format!(
            "选择的选项: {}",
            response.selected_options.join(", ")
        ));
    }

    // 2. 处理用户输入文本
    if let Some(user_input) = response.user_input.as_ref() {
        if !user_input.trim().is_empty() {
            if custom_selected {
                text_parts.push(format!("用户最终要求: {}", user_input.trim()));
            } else {
                text_parts.push(user_input.trim().to_string());
            }
        }
    }

    // 3. 处理条件上下文块。文本回退明确区分临时上下文与可写记忆，避免误触发 ji。
    let (transient_blocks, memory_blocks) = split_context_blocks(&response.context_blocks);
    if !transient_blocks.is_empty() {
        text_parts.push(format_context_block_section(
            "本轮临时上下文（禁止写入 ji）",
            &transient_blocks,
        ));
    }
    if !memory_blocks.is_empty() {
        text_parts.push(format_context_block_section(
            "可写记忆候选（仅这些内容允许按 category 调用 ji）",
            &memory_blocks,
        ));
    }

    // 4. 处理图片附件
    let mut image_info_parts = Vec::new();
    for (index, image) in response.images.iter().enumerate() {
        // 添加图片到结果中（图片在前）
        result.push(Content::image(image.data.clone(), image.media_type.clone()));

        // 生成图片信息
        let base64_len = image.data.len();
        let preview = if base64_len > 50 {
            format!("{}...", &image.data[..50])
        } else {
            image.data.clone()
        };

        // 计算图片大小
        let estimated_size = (base64_len * 3) / 4;
        let size_str = if estimated_size < 1024 {
            format!("{} B", estimated_size)
        } else if estimated_size < 1024 * 1024 {
            format!("{:.1} KB", estimated_size as f64 / 1024.0)
        } else {
            format!("{:.1} MB", estimated_size as f64 / (1024.0 * 1024.0))
        };

        let filename_info = image
            .filename
            .as_ref()
            .map(|f| format!("\n文件名: {}", f))
            .unwrap_or_default();

        let image_info = format!(
            "=== 图片 {} ==={}\n类型: {}\n大小: {}\nBase64 预览: {}\n完整 Base64 长度: {} 字符",
            index + 1,
            filename_info,
            image.media_type,
            size_str,
            preview,
            base64_len
        );
        image_info_parts.push(image_info);
    }

    // 5. 合并所有文本内容
    let mut all_text_parts = text_parts;
    all_text_parts.extend(image_info_parts);

    // 6. 添加兼容性说明
    if !response.images.is_empty() {
        all_text_parts.push(format!(
            "💡 注意：用户提供了 {} 张图片。如果 AI 助手无法显示图片，图片数据已包含在上述 Base64 信息中。",
            response.images.len()
        ));
    }

    // 7. 将文本内容添加到结果中（图片后面）
    if !all_text_parts.is_empty() {
        let combined_text = all_text_parts.join("\n\n");
        result.push(Content::text(combined_text));
    }

    // 8. 如果没有任何内容，添加默认响应
    if result.is_empty() {
        result.push(Content::text("用户未提供任何内容".to_string()));
    }

    let structured_content = build_structured_content(&response);

    Ok(ParsedMcpResponse {
        content: result,
        structured_content: Some(structured_content),
    })
}

fn split_context_blocks(
    blocks: &[ResponseContextBlock],
) -> (Vec<&ResponseContextBlock>, Vec<&ResponseContextBlock>) {
    let mut transient_blocks = Vec::new();
    let mut memory_blocks = Vec::new();

    for block in blocks {
        if block.content.trim().is_empty() {
            continue;
        }
        if block.normalized_memory_policy() == "save" {
            memory_blocks.push(block);
        } else {
            transient_blocks.push(block);
        }
    }

    (transient_blocks, memory_blocks)
}

fn format_context_block_section(title: &str, blocks: &[&ResponseContextBlock]) -> String {
    let mut lines = Vec::with_capacity(blocks.len() + 1);
    lines.push(format!("{}:", title));
    for block in blocks {
        let category = block
            .normalized_memory_category()
            .map(|category| format!(" category={}", category))
            .unwrap_or_default();
        lines.push(format!(
            "- [{}{}] {}",
            &block.scope,
            category,
            block.content.trim()
        ));
    }
    lines.join("\n")
}

fn build_structured_content(response: &McpResponse) -> Value {
    let memory_actions: Vec<Value> = response
        .context_blocks
        .iter()
        .filter(|block| block.normalized_memory_policy() == "save")
        .map(|block| {
            serde_json::json!({
                "action": "记忆",
                "category": block.normalized_memory_category().unwrap_or("context"),
                "content": block.content.trim(),
                "source": {
                    "kind": &block.kind,
                    "id": &block.source_id,
                    "name": &block.source_name,
                    "scope": &block.scope
                }
            })
        })
        .collect();

    let transient_context: Vec<Value> = response
        .context_blocks
        .iter()
        .filter(|block| block.normalized_memory_policy() != "save")
        .map(|block| {
            serde_json::json!({
                "kind": &block.kind,
                "scope": &block.scope,
                "content": block.content.trim(),
                "source_id": &block.source_id,
                "source_name": &block.source_name
            })
        })
        .collect();

    serde_json::json!({
        "user_input": &response.user_input,
        "selected_options": &response.selected_options,
        "context_blocks": &response.context_blocks,
        "memory_intent": &response.memory_intent,
        "memory_actions": memory_actions,
        "transient_context": transient_context,
        "metadata": &response.metadata
    })
}

#[cfg(test)]
mod tests {
    use super::parse_mcp_response;
    use rmcp::model::RawContent;

    fn extract_text(response: &str) -> String {
        let content = parse_mcp_response(response).expect("响应应可解析");
        content
            .iter()
            .find_map(|item| match &item.raw {
                RawContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .expect("响应应包含文本内容")
    }

    #[test]
    fn custom_choice_promotes_user_input_as_final_requirement() {
        let response = serde_json::json!({
            "user_input": "不要按选项一执行，改为先补需求访谈。",
            "selected_options": ["其他：自定义要求"],
            "images": [],
            "metadata": {
                "timestamp": "2026-05-17T00:00:00Z",
                "request_id": "test",
                "source": "popup"
            }
        });

        let text = extract_text(&response.to_string());

        assert!(text.contains("用户选择了自定义要求"));
        assert!(text.contains("用户最终要求: 不要按选项一执行，改为先补需求访谈。"));
        assert!(!text.contains("选择的选项: 其他：自定义要求"));
    }

    #[test]
    fn normal_choice_keeps_existing_response_shape() {
        let response = serde_json::json!({
            "user_input": "补充说明",
            "selected_options": ["方案 A"],
            "images": [],
            "metadata": {
                "timestamp": "2026-05-17T00:00:00Z",
                "request_id": "test",
                "source": "popup"
            }
        });

        let text = extract_text(&response.to_string());

        assert!(text.contains("选择的选项: 方案 A"));
        assert!(text.contains("补充说明"));
        assert!(!text.contains("用户最终要求"));
    }
}
