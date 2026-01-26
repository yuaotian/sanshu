use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt, RoleServer,
    model::{ErrorData as McpError},
    transport::stdio,
    service::{RequestContext, ServerInitializeError},
};
use rmcp::model::*;
use std::collections::HashMap;

use super::tools::{InteractionTool, MemoryTool, AcemcpTool, Context7Tool, IconTool, SkillsTool, UiuxTool, EnhanceTool};
use super::types::{ZhiRequest, JiyiRequest, TuRequest, SkillRunRequest};
use crate::mcp::tools::enhance::mcp::EnhanceMcpRequest;
use crate::mcp::tools::context7::types::Context7Request;
use crate::config::load_standalone_config;
use crate::{log_important, log_debug};

#[derive(Clone)]
pub struct ZhiServer {
    enabled_tools: HashMap<String, bool>,
}

impl Default for ZhiServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ZhiServer {
    pub fn new() -> Self {
        // 尝试加载配置，如果失败则使用默认配置
        let enabled_tools = match load_standalone_config() {
            Ok(config) => config.mcp_config.tools,
            Err(e) => {
                log_important!(warn, "无法加载配置文件，使用默认工具配置: {}", e);
                crate::config::default_mcp_tools()
            }
        };

        Self { enabled_tools }
    }

    /// 检查工具是否启用 - 动态读取最新配置
    fn is_tool_enabled(&self, tool_name: &str) -> bool {
        // 每次都重新读取配置，确保获取最新状态
        match load_standalone_config() {
            Ok(config) => {
                let enabled = config.mcp_config.tools.get(tool_name).copied().unwrap_or(true);
                log_debug!("工具 {} 当前状态: {}", tool_name, enabled);
                enabled
            }
            Err(e) => {
                log_important!(warn, "读取配置失败，使用缓存状态: {}", e);
                // 如果读取失败，使用缓存的配置
                self.enabled_tools.get(tool_name).copied().unwrap_or(true)
            }
        }
    }
}

impl ServerHandler for ZhiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "Zhi-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                title: None,
                website_url: None,
            },
            instructions: Some("Zhi 智能代码审查工具，支持交互式对话和记忆管理".to_string()),
        }
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ServerInfo, McpError> {
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        use std::sync::Arc;
        use std::borrow::Cow;

        let mut tools = Vec::new();

        // 三术工具始终可用（必需工具）
        let zhi_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "要显示给用户的消息"
                },
                "predefined_options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "预定义的选项列表（可选）"
                },
                "is_markdown": {
                    "type": "boolean",
                    "description": "消息是否为Markdown格式，默认为true"
                }
            },
            "required": ["message"]
        });

        if let serde_json::Value::Object(schema_map) = zhi_schema {
            tools.push(Tool {
                name: Cow::Borrowed("zhi"),
                description: Some(Cow::Borrowed("智能代码审查交互工具，支持预定义选项、自由文本输入和图片上传")),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            });
        }

        // 记忆管理工具 - 仅在启用时添加
        if self.is_tool_enabled("ji") {
            let ji_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "操作类型：记忆(添加) | 回忆(查询) | 整理(去重) | 列表(全部记忆) | 预览相似(检测相似度) | 配置(获取/更新) | 删除(移除记忆)"
                    },
                    "project_path": {
                        "type": "string",
                        "description": "项目路径（必需）"
                    },
                    "content": {
                        "type": "string",
                        "description": "记忆内容（记忆/预览相似操作时必需）"
                    },
                    "category": {
                        "type": "string",
                        "description": "记忆分类：rule(规范规则), preference(用户偏好), pattern(最佳实践), context(项目上下文)"
                    },
                    "config": {
                        "type": "object",
                        "description": "配置参数（配置操作时使用）",
                        "properties": {
                            "similarity_threshold": {
                                "type": "number",
                                "description": "相似度阈值 (0.5~0.95)，超过此值视为重复"
                            },
                            "dedup_on_startup": {
                                "type": "boolean",
                                "description": "启动时自动去重"
                            },
                            "enable_dedup": {
                                "type": "boolean",
                                "description": "启用去重检测"
                            }
                        }
                    },
                    "memory_id": {
                        "type": "string",
                        "description": "记忆ID（删除操作时必需）"
                    }
                },
                "required": ["action", "project_path"]
            });

            if let serde_json::Value::Object(schema_map) = ji_schema {
                tools.push(Tool {
                    name: Cow::Borrowed("ji"),
                    description: Some(Cow::Borrowed("全局记忆管理工具，用于存储和管理重要的开发规范、用户偏好和最佳实践")),
                    input_schema: Arc::new(schema_map),
                    annotations: None,
                    icons: None,
                    meta: None,
                    output_schema: None,
                    title: None,
                });
            }
        }

        // 代码搜索工具 - 仅在启用时添加
        if self.is_tool_enabled("sou") {
            tools.push(AcemcpTool::get_tool_definition());
        }

        // Context7 文档查询工具 - 仅在启用时添加
        if self.is_tool_enabled("context7") {
            tools.push(Context7Tool::get_tool_definition());
        }

        // 图标工坊工具 - 仅在启用时添加
        if self.is_tool_enabled("icon") {
            tools.push(IconTool::get_tool_definition());
        }

        // UI/UX 工具 - 仅在启用时添加
        if self.is_tool_enabled("uiux") {
            tools.extend(UiuxTool::get_tool_definitions());
        }

        // 提示词增强工具 - 仅在启用时添加
        if self.is_tool_enabled("enhance") {
            tools.push(EnhanceTool::get_tool_definition());
        }

        // 技能运行时工具 - 动态发现 skills 并追加工具
        let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        tools.extend(SkillsTool::list_dynamic_tools(&project_root));

        log_debug!("返回给客户端的工具列表: {:?}", tools.iter().map(|t| &t.name).collect::<Vec<_>>());

        Ok(ListToolsResult {
            meta: None,
            next_cursor: None,
            tools,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        log_debug!("收到工具调用请求: {}", request.name);

        match request.name.as_ref() {
            "zhi" => {
                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let zhi_request: ZhiRequest = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                // 调用三术工具
                InteractionTool::zhi(zhi_request).await
            }
            "ji" => {
                // 检查记忆管理工具是否启用
                if !self.is_tool_enabled("ji") {
                    return Err(McpError::internal_error(
                        "记忆管理工具已被禁用".to_string(),
                        None
                    ));
                }

                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let ji_request: JiyiRequest = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                // 调用记忆工具
                MemoryTool::jiyi(ji_request).await
            }
            "sou" => {
                // 检查代码搜索工具是否启用
                if !self.is_tool_enabled("sou") {
                    return Err(McpError::internal_error(
                        "代码搜索工具已被禁用".to_string(),
                        None
                    ));
                }

                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                // 使用acemcp模块中的AcemcpRequest类型
                let acemcp_request: crate::mcp::tools::acemcp::types::AcemcpRequest = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                // 调用代码搜索工具
                AcemcpTool::search_context(acemcp_request).await
            }
            "context7" => {
                // 检查 Context7 工具是否启用
                if !self.is_tool_enabled("context7") {
                    return Err(McpError::internal_error(
                        "Context7 文档查询工具已被禁用".to_string(),
                        None
                    ));
                }

                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let context7_request: Context7Request = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                // 调用 Context7 工具
                Context7Tool::query_docs(context7_request).await
            }
            "tu" => {
                // 检查图标工坊工具是否启用
                if !self.is_tool_enabled("icon") {
                    return Err(McpError::internal_error(
                        "图标工坊工具已被禁用".to_string(),
                        None
                    ));
                }

                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let tu_request: TuRequest = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                // 调用图标工坊工具
                IconTool::tu(tu_request).await
            }
            // 兼容 Antigravity：UI/UX 工具名使用下划线分隔
            name if name.starts_with("uiux_") => {
                if !self.is_tool_enabled("uiux") {
                    return Err(McpError::internal_error(
                        "UI/UX 工具已被禁用".to_string(),
                        None
                    ));
                }

                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                UiuxTool::call_tool(name, arguments_value).await
            }
            name if name == "skill_run" || name.starts_with("skill_") => {
                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let skill_request: SkillRunRequest = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                SkillsTool::call_tool(name, skill_request, &project_root).await
            }
            "enhance" => {
                // 检查增强工具是否启用
                if !self.is_tool_enabled("enhance") {
                    return Err(McpError::internal_error(
                        "提示词增强工具已被禁用".to_string(),
                        None
                    ));
                }

                // 解析请求参数
                let arguments_value = request.arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let enhance_request: EnhanceMcpRequest = serde_json::from_value(arguments_value)
                    .map_err(|e| McpError::invalid_params(format!("参数解析失败: {}", e), None))?;

                // 调用提示词增强工具
                EnhanceTool::enhance(enhance_request).await
            }
            _ => {
                Err(McpError::invalid_request(
                    format!("未知的工具: {}", request.name),
                    None
                ))
            }
        }
    }
}



/// 启动MCP服务器
pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    // 创建并运行服务器
    let service = match ZhiServer::new().serve(stdio()).await {
        Ok(service) => service,
        Err(e) => {
            match &e {
                ServerInitializeError::ConnectionClosed(_) => {
                    log_important!(
                        error,
                        "启动服务器失败：初始化阶段连接已关闭。通常是未通过 MCP 客户端以 stdio 管道启动，或客户端启动后立即退出。请检查 MCP 客户端配置（command/args/stdio），不要直接双击运行。"
                    );
                }
                _ => {
                    log_important!(error, "启动服务器失败: {}", e);
                }
            }
            return Err(Box::new(e));
        }
    };

    // 等待服务器关闭
    service.waiting().await?;
    Ok(())
}
