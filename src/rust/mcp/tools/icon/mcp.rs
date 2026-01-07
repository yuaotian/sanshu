// MCP 工具定义
// 定义图标工坊作为 MCP 工具的元数据（可选：如果需要通过 MCP 协议暴露）

use rmcp::model::Tool;
use std::borrow::Cow;
use std::sync::Arc;

/// 图标工坊 MCP 工具
/// 
/// 注意：图标搜索功能主要通过 Tauri 命令供前端调用，
/// 此 MCP 工具定义是可选的，用于支持 AI 助手直接调用
pub struct IconTool;

impl IconTool {
    /// 获取工具定义
    /// 
    /// 返回 MCP 协议规范的工具定义，可用于 AI 助手调用
    pub fn get_tool_definition() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词（必填），用于在 Iconfont 图标库中搜索图标"
                },
                "style": {
                    "type": "string",
                    "enum": ["line", "fill", "flat", "all"],
                    "description": "图标风格：line(线性)、fill(面性)、flat(扁平)、all(全部)，默认 all"
                },
                "fills": {
                    "type": "string",
                    "enum": ["single", "multi", "all"],
                    "description": "填充类型：single(单色)、multi(多色)、all(全部)，默认 all"
                },
                "sort_type": {
                    "type": "string",
                    "enum": ["relate", "new", "hot"],
                    "description": "排序方式：relate(相关度)、new(最新)、hot(最热)，默认 relate"
                },
                "page": {
                    "type": "integer",
                    "description": "页码，默认 1"
                },
                "page_size": {
                    "type": "integer",
                    "description": "每页数量，默认 50，最大 100"
                }
            },
            "required": ["query"]
        });

        if let serde_json::Value::Object(schema_map) = schema {
            Tool {
                name: Cow::Borrowed("search_icons"),
                description: Some(Cow::Borrowed(
                    "搜索 Iconfont 图标库中的图标。输入关键词后返回匹配的图标列表，支持按风格、填充类型和排序方式筛选。"
                )),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some("图标搜索".to_string()),
            }
        } else {
            // 不应该到达这里
            panic!("无法创建 IconTool schema")
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
