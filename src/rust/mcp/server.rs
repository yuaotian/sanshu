use anyhow::Result;
use rmcp::model::*;
use rmcp::{
    model::ErrorData as McpError,
    service::{RequestContext, ServerInitializeError},
    transport::{async_rw::AsyncRwTransport, stdio, Transport},
    RoleServer, ServerHandler, ServiceExt,
};
use std::collections::HashMap;
use std::time::Instant;

use super::tools::{
    Context7Tool, EnhanceTool, IconTool, InteractionTool, MemoryTool, PlanTool, SkillsTool,
    SouTool, TavilyTool, UiuxTool,
};
use super::types::{JiyiRequest, SkillRunRequest, TuRequest, ZhiRequest};
use crate::config::load_standalone_config;
use crate::mcp::tools::context7::types::Context7Request;
use crate::mcp::tools::enhance::mcp::EnhanceMcpRequest;
use crate::mcp::tools::plan::PlanRequest;
use crate::mcp::tools::tavily::types::TavilyRequest;
use crate::mcp::utils::generate_request_id;
use crate::mcp::utils::safe_truncate_clean;
use crate::{log_debug, log_important};

const WINDSURF_ZHI_ALIAS: &str = "work_note";
const MCP_PROFILE_ENV: &str = "SANSHU_MCP_PROFILE";
const MCP_SERVER_DISCOVER_METHOD: &str = "server/discover";

/// 兼容会先探测新版生命周期、再回退旧版 initialize 的 MCP 客户端。
struct DiscoverFallbackTransport<T> {
    inner: T,
    check_initial_request: bool,
}

impl<T> DiscoverFallbackTransport<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            check_initial_request: true,
        }
    }
}

impl<T> Transport<RoleServer> for DiscoverFallbackTransport<T>
where
    T: Transport<RoleServer>,
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<ClientJsonRpcMessage>> + Send {
        async move {
            loop {
                let message = self.inner.receive().await?;
                if !self.check_initial_request {
                    return Some(message);
                }

                self.check_initial_request = false;
                let ClientJsonRpcMessage::Request(JsonRpcRequest {
                    id,
                    request: ClientRequest::CustomRequest(request),
                    ..
                }) = &message
                else {
                    return Some(message);
                };
                if request.method != MCP_SERVER_DISCOVER_METHOD {
                    return Some(message);
                }

                // 中文说明：返回“不支持”后保持 stdio 打开，让新版客户端回退发送旧版 initialize。
                let response = ServerJsonRpcMessage::error(
                    McpError::new(ErrorCode::METHOD_NOT_FOUND, "Method not found", None),
                    id.clone(),
                );
                if let Err(error) = self.inner.send(response).await {
                    log_important!(error, "回应 MCP server/discover 兼容探测失败: {}", error);
                    return None;
                }
                log_debug!("已拒绝首包 server/discover，等待客户端回退发送 initialize");
            }
        }
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum McpClientProfile {
    Standard,
    Windsurf,
}

impl McpClientProfile {
    fn detect() -> Self {
        if let Ok(raw) = std::env::var(MCP_PROFILE_ENV) {
            match raw.trim().to_ascii_lowercase().as_str() {
                "windsurf" | "compat" | "alias" => return Self::Windsurf,
                "standard" | "zhi" => return Self::Standard,
                "" => {}
                other => {
                    log_important!(
                        warn,
                        "未知 MCP profile: {}={}，将继续按可执行文件名自动判断",
                        MCP_PROFILE_ENV,
                        other
                    );
                }
            }
        }

        // 中文说明：`sanshu` 是面向不支持中文命令的 MCP 客户端的 ASCII 入口。
        if std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .map(|stem| stem.eq_ignore_ascii_case("sanshu"))
            .unwrap_or(false)
        {
            return Self::Windsurf;
        }

        Self::Standard
    }

    fn is_windsurf(self) -> bool {
        matches!(self, Self::Windsurf)
    }
}

#[derive(Clone)]
pub struct ZhiServer {
    enabled_tools: HashMap<String, bool>,
    mcp_profile: McpClientProfile,
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
        let mcp_profile = McpClientProfile::detect();
        log_important!(info, "MCP profile: {:?}", mcp_profile);

        Self {
            enabled_tools,
            mcp_profile,
        }
    }

    /// 检查工具是否启用 - 动态读取最新配置
    fn is_tool_enabled(&self, tool_name: &str) -> bool {
        // 每次都重新读取配置，确保获取最新状态
        match load_standalone_config() {
            Ok(config) => {
                let enabled = config
                    .mcp_config
                    .tools
                    .get(tool_name)
                    .copied()
                    .unwrap_or(true);
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

    fn zhi_public_tool_name(&self) -> &'static str {
        if self.mcp_profile.is_windsurf() {
            WINDSURF_ZHI_ALIAS
        } else {
            "zhi"
        }
    }

    fn zhi_public_title(&self) -> &'static str {
        if self.mcp_profile.is_windsurf() {
            "Work Note"
        } else {
            "代码审阅记录"
        }
    }

    fn zhi_public_description(&self) -> &'static str {
        "记录方案摘要、候选项与处理结果，返回结构化数据。方案选择场景应提供候选项；系统会为已有候选项补充“其他：自定义要求”兜底。"
    }

    fn is_zhi_entry(tool_name: &str) -> bool {
        tool_name == "zhi" || tool_name == WINDSURF_ZHI_ALIAS
    }
}

impl ServerHandler for ZhiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            // 中文说明：MCP 初始化元数据也可能被客户端侧规则扫描，这里保持中性表述。
            server_info: Implementation {
                name: "sanshu-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                title: None,
                website_url: None,
            },
            instructions: Some(
                "Sanshu MCP 服务，提供项目记录、上下文检索与辅助处理能力。".to_string(),
            ),
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
        use std::borrow::Cow;
        use std::sync::Arc;

        let mut tools = Vec::new();

        // 三术工具始终可用（必需工具）
        // 中文说明：对外 schema 使用中性字段名与描述，降低部分 MCP 客户端的内容级误判风险。
        let zhi_tool_name = self.zhi_public_tool_name();
        let zhi_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "brief": {
                    "type": "string",
                    "description": "审阅内容或方案摘要"
                },
                "choices": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "候选处理项列表（可选）。方案选择或确认场景建议提供 2-5 个明确选项；已有候选项时系统会自动追加“其他：自定义要求”兜底。"
                },
                "render_markdown": {
                    "type": "boolean",
                    "description": "是否按 Markdown 格式处理内容，默认 true"
                },
                "workspace": {
                    "type": "string",
                    "description": "工作区根目录绝对路径（必填）"
                },
                "agent_label": {
                    "type": "string",
                    "description": "AI 实例显示名称（可选，未提供时按请求短码回退）"
                }
            },
            "required": ["brief", "workspace"]
        });

        if let serde_json::Value::Object(schema_map) = zhi_schema {
            tools.push(Tool {
                name: Cow::Borrowed(zhi_tool_name),
                description: Some(Cow::Borrowed(self.zhi_public_description())),
                input_schema: Arc::new(schema_map),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: Some(self.zhi_public_title().to_string()),
            });
        }

        // 记忆管理工具 - 仅在启用时添加
        if self.is_tool_enabled("ji") {
            let ji_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "操作类型：记忆(添加) | 回忆(查询) | 整理(去重) | 预览整理(候选预览) | 应用整理(按计划清理) | 备份列表 | 恢复备份 | 导出备份 | 列表(全部记忆) | 预览相似(检测相似度) | 配置(获取/更新) | 删除(移除记忆)"
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
                            "upsert_threshold": {
                                "type": "number",
                                "description": "同类更新阈值 (0.4~0.9)，必须小于相似度阈值"
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
                    },
                    "threshold": {
                        "type": "number",
                        "description": "清理阈值（预览整理时可选，默认使用同类更新阈值）"
                    },
                    "categories": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "清理分类过滤（预览整理时可选）"
                    },
                    "include_cross_category": {
                        "type": "boolean",
                        "description": "是否允许跨分类清理（默认 false）"
                    },
                    "cleanup_plan": {
                        "type": "object",
                        "description": "应用整理计划（应用整理时必需）"
                    },
                    "backup_file": {
                        "type": "string",
                        "description": "备份文件名（恢复备份/导出备份时必需）"
                    }
                },
                "required": ["action", "project_path"]
            });

            if let serde_json::Value::Object(schema_map) = ji_schema {
                tools.push(Tool {
                    name: Cow::Borrowed("ji"),
                    description: Some(Cow::Borrowed(
                        "全局记忆管理工具，用于存储和管理重要的开发规范、用户偏好和最佳实践",
                    )),
                    input_schema: Arc::new(schema_map),
                    annotations: None,
                    icons: None,
                    meta: None,
                    output_schema: None,
                    title: None,
                });
            }
        }

        // 开发计划工具 - 仅在启用时添加
        if self.is_tool_enabled("plan") {
            tools.push(PlanTool::get_tool_definition());
        }

        // 代码搜索工具 - 仅在启用时添加
        if self.is_tool_enabled("sou") {
            tools.push(SouTool::get_tool_definition());
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

        // Tavily AI 搜索工具 - 仅在启用时添加
        if self.is_tool_enabled("tavily") {
            tools.push(TavilyTool::get_tool_definition());
        }

        // 技能运行时工具 - 动态发现 skills 并追加工具
        let project_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        tools.extend(SkillsTool::list_dynamic_tools(&project_root));

        log_debug!(
            "返回给客户端的工具列表: {:?}",
            tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );

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
        let call_id = generate_request_id();
        let start = Instant::now();

        let tool_name = request.name.to_string();
        let arg_keys: Vec<String> = request
            .arguments
            .as_ref()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();

        // 解析参数（保持与旧逻辑一致：None -> 空对象）
        let arguments_value = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        // 统一入口日志（全链路追踪用）
        log_important!(
            info,
            "[MCP] 调用开始: call_id={}, tool={}, arg_keys={:?}",
            call_id,
            tool_name,
            arg_keys
        );

        // 常见字段摘要（避免打印完整内容导致日志膨胀/泄露）
        if let Some(obj) = arguments_value.as_object() {
            for k in ["brief", "message", "prompt", "query", "content"] {
                if let Some(s) = obj.get(k).and_then(|v| v.as_str()) {
                    log_debug!(
                        "[MCP] 参数摘要: call_id={}, tool={}, {}_len={}, {}_preview={}",
                        call_id,
                        tool_name,
                        k,
                        s.len(),
                        k,
                        safe_truncate_clean(s, 200)
                    );
                }
            }
        }

        let result: Result<CallToolResult, McpError> = match tool_name.as_str() {
            tool if Self::is_zhi_entry(tool) => {
                match serde_json::from_value::<ZhiRequest>(arguments_value) {
                    Ok(zhi_request) => {
                        // 调用三术工具（将 call_id 作为 request.id 贯穿到 GUI/响应）
                        InteractionTool::zhi_with_request_id(zhi_request, call_id.clone()).await
                    }
                    Err(e) => {
                        log_important!(
                            warn,
                            "[MCP] 参数解析失败: call_id={}, tool={}, error={}",
                            call_id,
                            tool,
                            e
                        );
                        Err(McpError::invalid_params(
                            format!("参数解析失败: {}", e),
                            None,
                        ))
                    }
                }
            }
            "ji" => {
                if !self.is_tool_enabled("ji") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=ji", call_id);
                    Err(McpError::internal_error(
                        "记忆管理工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<JiyiRequest>(arguments_value) {
                        Ok(ji_request) => MemoryTool::jiyi(ji_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=ji, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            "plan" => {
                if !self.is_tool_enabled("plan") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=plan", call_id);
                    Err(McpError::internal_error(
                        "开发计划工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<PlanRequest>(arguments_value) {
                        Ok(plan_request) => PlanTool::execute(plan_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=plan, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            "sou" => {
                if !self.is_tool_enabled("sou") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=sou", call_id);
                    Err(McpError::internal_error(
                        "代码搜索工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<crate::mcp::tools::sou::SouRequest>(
                        arguments_value,
                    ) {
                        Ok(sou_request) => SouTool::search_context(sou_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=sou, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            "context7" => {
                if !self.is_tool_enabled("context7") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=context7", call_id);
                    Err(McpError::internal_error(
                        "Context7 文档查询工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<Context7Request>(arguments_value) {
                        Ok(context7_request) => Context7Tool::query_docs(context7_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=context7, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            "tu" => {
                if !self.is_tool_enabled("icon") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=tu(icon)", call_id);
                    Err(McpError::internal_error(
                        "图标工坊工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<TuRequest>(arguments_value) {
                        Ok(tu_request) => IconTool::tu(tu_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=tu, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            "uiux" => {
                if !self.is_tool_enabled("uiux") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=uiux", call_id);
                    Err(McpError::internal_error(
                        "UI/UX 工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    UiuxTool::call_tool("uiux", arguments_value).await
                }
            }
            name if name == "skill_run" || name.starts_with("skill_") => {
                match serde_json::from_value::<SkillRunRequest>(arguments_value) {
                    Ok(skill_request) => {
                        let project_root = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."));
                        SkillsTool::call_tool(name, skill_request, &project_root).await
                    }
                    Err(e) => {
                        log_important!(
                            warn,
                            "[MCP] 参数解析失败: call_id={}, tool={}, error={}",
                            call_id,
                            name,
                            e
                        );
                        Err(McpError::invalid_params(
                            format!("参数解析失败: {}", e),
                            None,
                        ))
                    }
                }
            }
            "enhance" => {
                if !self.is_tool_enabled("enhance") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=enhance", call_id);
                    Err(McpError::internal_error(
                        "提示词增强工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<EnhanceMcpRequest>(arguments_value) {
                        Ok(enhance_request) => EnhanceTool::enhance(enhance_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=enhance, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            "tavily" => {
                if !self.is_tool_enabled("tavily") {
                    log_important!(warn, "[MCP] 工具已禁用: call_id={}, tool=tavily", call_id);
                    Err(McpError::internal_error(
                        "Tavily AI 搜索工具已被禁用".to_string(),
                        None,
                    ))
                } else {
                    match serde_json::from_value::<TavilyRequest>(arguments_value) {
                        Ok(tavily_request) => TavilyTool::execute(tavily_request).await,
                        Err(e) => {
                            log_important!(
                                warn,
                                "[MCP] 参数解析失败: call_id={}, tool=tavily, error={}",
                                call_id,
                                e
                            );
                            Err(McpError::invalid_params(
                                format!("参数解析失败: {}", e),
                                None,
                            ))
                        }
                    }
                }
            }
            _ => Err(McpError::invalid_request(
                format!("未知的工具: {}", tool_name),
                None,
            )),
        };

        // 统一出口日志（全链路追踪用）
        let elapsed_ms = start.elapsed().as_millis();
        match &result {
            Ok(r) => {
                let is_error = r.is_error.unwrap_or(false);
                log_important!(
                    info,
                    "[MCP] 调用结束: call_id={}, tool={}, is_error={}, content_items={}, elapsed_ms={}",
                    call_id,
                    tool_name,
                    is_error,
                    r.content.len(),
                    elapsed_ms
                );
            }
            Err(e) => {
                log_important!(
                    error,
                    "[MCP] 调用失败: call_id={}, tool={}, elapsed_ms={}, error={}",
                    call_id,
                    tool_name,
                    elapsed_ms,
                    e
                );
            }
        }

        result
    }
}

/// 启动MCP服务器
pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    // 创建并运行服务器
    let (stdin, stdout) = stdio();
    let transport = DiscoverFallbackTransport::new(AsyncRwTransport::new_server(stdin, stdout));
    let service = match ZhiServer::new().serve(transport).await {
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

    // 中文说明：MCP 进程长期维护代码监听；握手成功后再恢复，避免探测失败产生批量取消日志。
    start_acemcp_watch_config_sync();

    // 等待服务器关闭
    service.waiting().await?;
    Ok(())
}

fn start_acemcp_watch_config_sync() {
    tokio::spawn(async {
        let watcher_manager = crate::mcp::tools::acemcp::watcher::get_watcher_manager();
        watcher_manager.sync_with_persisted_watch_projects().await;
        if let Err(error) = crate::mcp::tools::acemcp::mcp::resume_index_jobs().await {
            log_important!(warn, "恢复 ACE 未完成索引任务失败: {}", error);
        }

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            interval.tick().await;
            watcher_manager.sync_with_persisted_watch_projects().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    struct MockTransport {
        incoming: VecDeque<ClientJsonRpcMessage>,
        sent: Arc<Mutex<Vec<ServerJsonRpcMessage>>>,
    }

    impl MockTransport {
        fn new(incoming: Vec<ClientJsonRpcMessage>) -> Self {
            Self {
                incoming: incoming.into(),
                sent: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl Transport<RoleServer> for MockTransport {
        type Error = Infallible;

        fn send(
            &mut self,
            item: ServerJsonRpcMessage,
        ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
            let sent = Arc::clone(&self.sent);
            async move {
                sent.lock().expect("sent lock poisoned").push(item);
                Ok(())
            }
        }

        fn receive(
            &mut self,
        ) -> impl std::future::Future<Output = Option<ClientJsonRpcMessage>> + Send {
            std::future::ready(self.incoming.pop_front())
        }

        fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
            std::future::ready(Ok(()))
        }
    }

    fn client_message(value: serde_json::Value) -> ClientJsonRpcMessage {
        serde_json::from_value(value).expect("valid client JSON-RPC message")
    }

    fn discover_request(id: u64) -> ClientJsonRpcMessage {
        client_message(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": MCP_SERVER_DISCOVER_METHOD,
            "params": {}
        }))
    }

    fn initialize_request(id: u64) -> ClientJsonRpcMessage {
        client_message(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "transport-test",
                    "version": "1.0.0"
                }
            }
        }))
    }

    fn assert_client_message_eq(
        actual: Option<ClientJsonRpcMessage>,
        expected: &ClientJsonRpcMessage,
    ) {
        let actual = actual.expect("expected client JSON-RPC message");
        assert_eq!(
            serde_json::to_value(actual).expect("serialize actual message"),
            serde_json::to_value(expected).expect("serialize expected message")
        );
    }

    #[tokio::test]
    async fn initial_discover_returns_method_not_found_then_forwards_initialize() {
        let initialize = initialize_request(2);
        let mock = MockTransport::new(vec![discover_request(1), initialize.clone()]);
        let sent = Arc::clone(&mock.sent);
        let mut transport = DiscoverFallbackTransport::new(mock);

        assert_client_message_eq(transport.receive().await, &initialize);

        let sent = sent.lock().expect("sent lock poisoned");
        assert_eq!(sent.len(), 1);
        let ServerJsonRpcMessage::Error(error) = &sent[0] else {
            panic!("expected method-not-found error");
        };
        assert_eq!(error.id, RequestId::Number(1));
        assert_eq!(error.error.code, ErrorCode::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn legacy_initialize_is_forwarded_without_response() {
        let initialize = initialize_request(1);
        let mock = MockTransport::new(vec![initialize.clone()]);
        let sent = Arc::clone(&mock.sent);
        let mut transport = DiscoverFallbackTransport::new(mock);

        assert_client_message_eq(transport.receive().await, &initialize);
        assert!(sent.lock().expect("sent lock poisoned").is_empty());
    }

    #[tokio::test]
    async fn discover_after_initial_request_is_not_intercepted() {
        let initialize = initialize_request(1);
        let discover = discover_request(2);
        let mock = MockTransport::new(vec![initialize.clone(), discover.clone()]);
        let sent = Arc::clone(&mock.sent);
        let mut transport = DiscoverFallbackTransport::new(mock);

        assert_client_message_eq(transport.receive().await, &initialize);
        assert_client_message_eq(transport.receive().await, &discover);
        assert!(sent.lock().expect("sent lock poisoned").is_empty());
    }
}
