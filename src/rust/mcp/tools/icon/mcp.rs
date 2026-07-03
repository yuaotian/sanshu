// MCP 工具定义
// 定义图标工坊作为 MCP 工具的元数据

use rmcp::model::{CallToolResult, ErrorData as McpError, Tool};
use std::borrow::Cow;
use std::sync::Arc;

use crate::mcp::handlers::create_icon_popup;
use crate::mcp::types::TuRequest;

/// 图标工坊 MCP 工具
///
/// 提供交互式图标选择功能，通过弹窗让用户搜索、预览、选择并保存图标
pub struct IconTool;

impl IconTool {
    /// 获取 "tu" 工具定义（交互式图标选择）
    ///
    /// schema 由 TuRequest 的 schemars 派生自动生成，避免手写 JSON 与类型定义漂移
    pub fn get_tool_definition() -> Tool {
        let schema = schemars::schema_for!(TuRequest);
        let schema_value =
            serde_json::to_value(schema).expect("TuRequest schema 序列化不应失败");

        if let serde_json::Value::Object(schema_map) = schema_value {
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
    /// 调用 GUI 进程，让用户在可视化界面中选择和保存图标；
    /// 按结构化响应的 status 字段区分 保存成功/用户取消/GUI错误
    pub async fn tu(request: TuRequest) -> Result<CallToolResult, McpError> {
        match create_icon_popup(&request).await {
            Ok(response) => match response.status.as_str() {
                "cancelled" => Ok(CallToolResult::success(vec![
                    rmcp::model::Content::text("用户取消了图标选择操作"),
                ])),
                "error" => {
                    let mut message = format!(
                        "图标选择失败: {}",
                        response.error.as_deref().unwrap_or("GUI 侧未知错误")
                    );
                    if response.saved_count > 0 {
                        message.push_str(&format!(
                            "（已保存 {} 个图标: {}）",
                            response.saved_count,
                            response.saved_names.join(", ")
                        ));
                    }
                    Err(McpError::internal_error(message, None))
                }
                "saved" if response.saved_count == 0 => Ok(CallToolResult::success(vec![
                    rmcp::model::Content::text("用户未选择任何图标"),
                ])),
                "saved" => {
                    // 构建详细的成功消息
                    let message = format!(
                        "✅ 已成功保存 {} 个图标到 {}\n\n保存的图标：\n{}",
                        response.saved_count,
                        response.save_path,
                        response
                            .saved_names
                            .iter()
                            .map(|name| format!("• {}", name))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                        message,
                    )]))
                }
                other => Err(McpError::internal_error(
                    format!("图标选择失败: 未知响应状态 {}", other),
                    None,
                )),
            },
            Err(e) => Err(McpError::internal_error(
                format!("图标选择失败: {}", e),
                None,
            )),
        }
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
