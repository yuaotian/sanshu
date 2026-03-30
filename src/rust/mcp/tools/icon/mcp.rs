// MCP 工具定义
// 定义图标工坊作为 MCP 工具的元数据

use rmcp::model::{Tool, CallToolResult, ErrorData as McpError};
use std::borrow::Cow;
use std::sync::Arc;

use crate::mcp::types::{TuRequest, PopupRequest, IconSaveResponse};
use crate::mcp::handlers::create_tauri_popup;
use crate::mcp::utils::{generate_request_id, safe_truncate_clean};

/// 图标工坊 MCP 工具
/// 
/// 提供交互式图标选择功能，通过弹窗让用户搜索、预览、选择并保存图标
pub struct IconTool;

impl IconTool {
    /// 获取 "tu" 工具定义（交互式图标选择）
    /// 
    /// 返回 MCP 协议规范的工具定义
    pub fn get_tool_definition() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "预设的搜索关键词（可选，用户可在界面中修改）"
                },
                "style": {
                    "type": "string",
                    "enum": ["line", "fill", "flat", "all"],
                    "description": "预设的图标风格：line(线性)、fill(面性)、flat(扁平)、all(全部)"
                },
                "save_path": {
                    "type": "string",
                    "description": "建议的保存路径（相对于项目根目录，如 assets/icons）"
                },
                "project_root": {
                    "type": "string",
                    "description": "项目根目录路径（用于计算相对路径）"
                }
            }
        });

        if let serde_json::Value::Object(schema_map) = schema {
            Tool {
                name: Cow::Borrowed("tu"),
                description: Some(Cow::Borrowed(
                    "交互式图标选择工具。打开可视化界面让用户搜索、预览、选择并保存 Iconfont 图标。支持筛选风格、分页浏览和批量保存。"
                )),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("图标工坊".to_string()),
            }
        } else {
            panic!("无法创建 IconTool schema")
        }
    }

    /// 执行 "tu" 工具 - 打开交互式图标选择弹窗
    ///
    /// 通过统一的 `--mcp-request` 通道拉起 GUI，用户选择图标后结果通过 stdout 返回
    pub async fn tu(request: TuRequest) -> Result<CallToolResult, McpError> {
        let popup_request = PopupRequest::Icon {
            id: generate_request_id(),
            project_root_path: request.project_root.clone(),
            query: request.query.clone(),
            style: request.style.clone(),
            save_path: request.save_path.clone(),
        };

        let response_str = create_tauri_popup(&popup_request).map_err(|e| {
            McpError::internal_error(format!("图标选择失败: {}", e), None)
        })?;

        let response: IconSaveResponse = serde_json::from_str(&response_str).map_err(|_| {
            if response_str.contains("取消") {
                return McpError::internal_error("用户取消了图标选择操作".to_string(), None);
            }
            McpError::internal_error(
                format!("解析图标响应失败，原始内容: {}", safe_truncate_clean(&response_str, 200)),
                None,
            )
        })?;

        if response.cancelled {
            return Ok(CallToolResult::success(vec![
                rmcp::model::Content::text("用户取消了图标选择操作"),
            ]));
        }

        if response.saved_count == 0 {
            return Ok(CallToolResult::success(vec![
                rmcp::model::Content::text("用户未选择任何图标"),
            ]));
        }

        let paths_section = if !response.saved_paths.is_empty() {
            response.saved_paths.iter()
                .map(|p| format!("  - `{}`", p))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            response.saved_names.iter()
                .map(|name| format!("  - {}", name))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let mut message = format!(
            "已保存 {} 个图标到 `{}`\n\n文件列表：\n{}",
            response.saved_count,
            response.save_path,
            paths_section,
        );

        if let Some(ref err) = response.error_message {
            message.push_str(&format!("\n\n⚠️ 部分错误: {}", err));
        }

        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(message),
        ]))
    }

    /// 获取图标工坊的工具信息（用于前端工具列表展示）
    pub fn get_tool_info() -> IconToolInfo {
        IconToolInfo {
            id: "icon".to_string(),
            name: "图标工坊".to_string(),
            description: "搜索和管理 Iconfont 图标，支持预览、复制和下载".to_string(),
            icon: "i-carbon-image".to_string(),
            icon_bg: "bg-purple-500/10".to_string(),
            dark_icon_bg: "dark:bg-purple-500/20".to_string(),
            enabled: true,
            can_disable: true,
            has_config: true,
        }
    }
}

/// 图标工具信息（用于前端展示）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IconToolInfo {
    /// 工具 ID
    pub id: String,
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 图标类名
    pub icon: String,
    /// 图标背景类名（亮色模式）
    pub icon_bg: String,
    /// 图标背景类名（暗色模式）
    pub dark_icon_bg: String,
    /// 是否启用
    pub enabled: bool,
    /// 是否可禁用
    pub can_disable: bool,
    /// 是否有配置项
    pub has_config: bool,
}

