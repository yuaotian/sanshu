use chrono;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ZhiRequest {
    #[schemars(description = "要显示给用户的消息")]
    pub message: String,
    #[schemars(description = "预定义的选项列表（可选）")]
    #[serde(default)]
    pub predefined_options: Vec<String>,
    #[schemars(description = "消息是否为Markdown格式，默认为true")]
    #[serde(default = "default_is_markdown")]
    pub is_markdown: bool,
    #[schemars(description = "项目根路径（可选，用于索引状态可视化）")]
    #[serde(default)]
    pub project_root_path: Option<String>,
}

fn default_is_markdown() -> bool {
    true
}

/// 记忆配置请求结构
/// 用于通过 MCP 或 Tauri 命令动态调整记忆去重配置
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MemoryConfigRequest {
    #[schemars(description = "相似度阈值 (0.5~0.95)，超过此值视为重复")]
    pub similarity_threshold: Option<f64>,
    #[schemars(description = "启动时自动去重")]
    pub dedup_on_startup: Option<bool>,
    #[schemars(description = "启用去重检测")]
    pub enable_dedup: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct JiyiRequest {
    #[schemars(description = "操作类型：记忆(添加) | 回忆(查询) | 整理(去重) | 列表(全部记忆) | 预览相似(检测相似度) | 配置(获取/更新) | 删除(移除记忆)")]
    pub action: String,
    #[schemars(description = "项目路径（必需）")]
    pub project_path: String,
    #[schemars(description = "记忆内容（记忆/预览相似操作时必需）")]
    #[serde(default)]
    pub content: String,
    #[schemars(
        description = "记忆分类：rule(规范规则), preference(用户偏好), pattern(最佳实践), context(项目上下文)"
    )]
    #[serde(default = "default_category")]
    pub category: String,
    #[schemars(description = "配置参数（配置操作时使用）")]
    #[serde(default)]
    pub config: Option<MemoryConfigRequest>,
    #[schemars(description = "记忆ID（删除操作时必需）")]
    #[serde(default)]
    pub memory_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AcemcpRequest {
    #[schemars(description = "项目根目录的绝对路径，使用正斜杠(/)作为分隔符")]
    pub project_root_path: String,
    #[schemars(description = "用于查找相关代码上下文的自然语言搜索查询")]
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SkillRunRequest {
    #[schemars(description = "技能名称（仅 skill_run 需要）")]
    #[serde(default)]
    pub skill_name: Option<String>,
    #[schemars(description = "动作名称（如 search/design_system/custom）")]
    #[serde(default)]
    pub action: Option<String>,
    #[schemars(description = "查询或输入（可选）")]
    #[serde(default)]
    pub query: Option<String>,
    #[schemars(description = "追加参数（可选）")]
    #[serde(default)]
    pub args: Option<Vec<String>>,
}

fn default_category() -> String {
    "context".to_string()
}

// ============ 图标工坊 MCP 请求类型 ============

/// 图标工坊交互请求（"tu" 工具）
/// 
/// 打开可视化图标选择界面，让用户搜索、预览、选择并保存图标
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TuRequest {
    /// 预设的搜索关键词（可选，用户可在界面中修改）
    #[schemars(description = "预设的搜索关键词（可选）")]
    #[serde(default)]
    pub query: Option<String>,
    
    /// 预设的图标风格：line(线性) | fill(面性) | flat(扁平) | all(全部)
    #[schemars(description = "预设的图标风格：line/fill/flat/all")]
    #[serde(default)]
    pub style: Option<String>,
    
    /// 建议的保存路径（可选，用户可修改）
    #[schemars(description = "建议的保存路径（相对于项目根目录）")]
    #[serde(default)]
    pub save_path: Option<String>,
    
    /// 项目根目录路径（用于计算相对路径）
    #[schemars(description = "项目根目录路径")]
    #[serde(default)]
    pub project_root: Option<String>,
}

/// 图标保存结果响应
#[derive(Debug, Serialize, Deserialize)]
pub struct IconSaveResponse {
    /// 保存的图标数量
    pub saved_count: u32,
    /// 保存路径
    pub save_path: String,
    /// 保存的图标名称列表
    pub saved_names: Vec<String>,
    /// 用户是否取消
    pub cancelled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PopupRequest {
    pub id: String,
    pub message: String,
    pub predefined_options: Option<Vec<String>>,
    pub is_markdown: bool,
    pub project_root_path: Option<String>,
}

/// 新的结构化响应数据格式
#[derive(Debug, Deserialize)]
pub struct McpResponse {
    pub user_input: Option<String>,
    pub selected_options: Vec<String>,
    pub images: Vec<ImageAttachment>,
    pub metadata: ResponseMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
    pub filename: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMetadata {
    pub timestamp: Option<String>,
    pub request_id: Option<String>,
    pub source: Option<String>,
}

/// 旧格式兼容性支持
#[derive(Debug, Deserialize)]
pub struct McpResponseContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
    pub source: Option<ImageSource>,
}

#[derive(Debug, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// 统一的响应构建函数
///
/// 用于生成标准的JSON响应格式，确保无GUI和有GUI模式输出一致
pub fn build_mcp_response(
    user_input: Option<String>,
    selected_options: Vec<String>,
    images: Vec<ImageAttachment>,
    request_id: Option<String>,
    source: &str,
) -> serde_json::Value {
    serde_json::json!({
        "user_input": user_input,
        "selected_options": selected_options,
        "images": images,
        "metadata": {
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "request_id": request_id,
            "source": source
        }
    })
}

/// 构建发送操作的响应
pub fn build_send_response(
    user_input: Option<String>,
    selected_options: Vec<String>,
    images: Vec<ImageAttachment>,
    request_id: Option<String>,
    source: &str,
) -> String {
    let response = build_mcp_response(user_input, selected_options, images, request_id, source);
    response.to_string()
}

/// 构建继续操作的响应
pub fn build_continue_response(request_id: Option<String>, source: &str) -> String {
    // 动态获取继续提示词
    let continue_prompt = if let Ok(config) = crate::config::load_standalone_config() {
        config.reply_config.continue_prompt
    } else {
        "请按照最佳实践继续".to_string()
    };

    let response = build_mcp_response(Some(continue_prompt), vec![], vec![], request_id, source);
    response.to_string()
}
