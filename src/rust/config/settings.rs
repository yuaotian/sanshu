use crate::constants::{audio, font, mcp, telegram, theme, window};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    #[serde(default = "default_ui_config")]
    pub ui_config: UiConfig, // UI相关配置（主题、窗口、置顶等）
    #[serde(default = "default_audio_config")]
    pub audio_config: AudioConfig, // 音频相关配置
    #[serde(default = "default_reply_config")]
    pub reply_config: ReplyConfig, // 继续回复配置
    #[serde(default = "default_mcp_config")]
    pub mcp_config: McpConfig, // MCP工具配置
    #[serde(default = "default_telegram_config")]
    pub telegram_config: TelegramConfig, // Telegram Bot配置
    #[serde(default = "default_custom_prompt_config")]
    pub custom_prompt_config: CustomPromptConfig, // 自定义prompt配置
    #[serde(default = "default_shortcut_config")]
    pub shortcut_config: ShortcutConfig, // 自定义快捷键配置
    #[serde(default = "default_proxy_config")]
    pub proxy_config: ProxyConfig, // 代理配置
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UiConfig {
    // 主题设置
    #[serde(default = "default_theme")]
    pub theme: String, // "light", "dark"

    // 字体设置
    #[serde(default = "default_font_config")]
    pub font_config: FontConfig,

    // 窗口设置
    #[serde(default = "default_window_config")]
    pub window_config: WindowConfig,

    // 置顶设置
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,

    // 代码高亮主题 ("auto", "github", "github-dark", "monokai", "atom-one-dark", "vs2015")
    #[serde(default = "default_hljs_theme")]
    pub hljs_theme: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FontConfig {
    // 字体系列
    #[serde(default = "default_font_family")]
    pub font_family: String, // "inter", "jetbrains-mono", "system", "custom"

    // 字体大小
    #[serde(default = "default_font_size")]
    pub font_size: String, // "small", "medium", "large"

    // 自定义字体系列（当 font_family 为 "custom" 时使用）
    #[serde(default = "default_custom_font_family")]
    pub custom_font_family: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WindowConfig {
    // 窗口约束设置
    #[serde(default = "default_auto_resize")]
    pub auto_resize: bool,
    #[serde(default = "default_max_width")]
    pub max_width: f64,
    #[serde(default = "default_max_height")]
    pub max_height: f64,
    #[serde(default = "default_min_width")]
    pub min_width: f64,
    #[serde(default = "default_min_height")]
    pub min_height: f64,

    // 当前模式
    #[serde(default = "default_window_fixed")]
    pub fixed: bool,

    // 固定模式的尺寸设置
    #[serde(default = "default_fixed_width")]
    pub fixed_width: f64,
    #[serde(default = "default_fixed_height")]
    pub fixed_height: f64,

    // 自由拉伸模式的尺寸设置
    #[serde(default = "default_free_width")]
    pub free_width: f64,
    #[serde(default = "default_free_height")]
    pub free_height: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioConfig {
    #[serde(default = "default_audio_notification_enabled")]
    pub notification_enabled: bool,
    #[serde(default = "default_audio_url")]
    pub custom_url: String, // 自定义音效URL
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReplyConfig {
    #[serde(default = "default_enable_continue_reply")]
    pub enable_continue_reply: bool,
    #[serde(default = "default_auto_continue_threshold")]
    pub auto_continue_threshold: u32, // 字符数阈值
    #[serde(default = "default_continue_prompt")]
    pub continue_prompt: String, // 继续回复的提示词
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpConfig {
    #[serde(default = "default_mcp_tools")]
    pub tools: HashMap<String, bool>, // MCP工具启用状态
    pub acemcp_base_url: Option<String>, // acemcp API端点URL
    pub acemcp_token: Option<String>,    // acemcp认证令牌
    pub acemcp_batch_size: Option<u32>,  // acemcp批处理大小
    pub acemcp_max_lines_per_blob: Option<u32>, // acemcp最大行数/块
    pub acemcp_text_extensions: Option<Vec<String>>, // acemcp文件扩展名
    pub acemcp_exclude_patterns: Option<Vec<String>>, // acemcp排除模式
    pub acemcp_watch_debounce_ms: Option<u64>, // 文件监听静默期（毫秒），默认 30000 (30秒)
    /// 文件监听最大等待时间（毫秒）：自首个事件起累积超过该值会强制 flush，
    /// 防止持续小写入导致永远触发不了同步。默认 300000 (5分钟)。
    pub acemcp_watch_max_wait_ms: Option<u64>,
    pub acemcp_auto_index_enabled: Option<bool>, // 全局自动索引开关（默认启用）
    /// 需要由 MCP 进程长期维护文件监听的项目路径列表。
    /// GUI 只负责写入监听意图，真正的长期 watcher 由三术 MCP 进程恢复和同步。
    pub acemcp_watched_projects: Option<Vec<String>>,
    /// 是否自动索引嵌套的 Git 子项目（默认启用）
    /// 当对父目录触发索引或监听时，自动检测并索引/监听所有子项目
    pub acemcp_index_nested_projects: Option<bool>,
    // Sou 代理配置
    pub acemcp_proxy_enabled: Option<bool>,    // 代理启用开关
    pub acemcp_proxy_host: Option<String>,     // 代理主机地址
    pub acemcp_proxy_port: Option<u16>,        // 代理端口
    pub acemcp_proxy_type: Option<String>,     // 代理类型: "http" | "https" | "socks5"
    pub acemcp_proxy_username: Option<String>, // 代理用户名（可选）
    pub acemcp_proxy_password: Option<String>, // 代理密码（可选）
    // Sou 多后端配置
    pub sou_default_backend: Option<String>, // "auto" | "ace" | "fast_context" | "both"
    pub sou_auto_order: Option<Vec<String>>, // auto 模式下的后端优先级
    pub sou_include_backend_headers: Option<bool>, // 是否在结果中标注后端来源
    pub sou_include_failed_backend_errors: Option<bool>, // 部分成功时是否附加失败后端诊断
    // Fast Context 配置
    pub fast_context_command: Option<String>, // 兼容旧配置：Rust 原生 fast-context 已不再使用
    pub fast_context_script_path: Option<String>, // 兼容旧配置：Rust 原生 fast-context 已不再使用
    pub fast_context_api_key: Option<String>, // Devin / Windsurf API Key，留空时走环境变量/自动发现
    pub fast_context_tree_depth: Option<u8>,  // 目录树深度 1-6
    pub fast_context_max_turns: Option<u8>,   // 搜索轮数 1-5
    pub fast_context_max_results: Option<u8>, // 返回文件数 1-30
    pub fast_context_max_commands: Option<u8>, // 每轮最大命令数
    pub fast_context_timeout_ms: Option<u64>, // 请求超时
    pub fast_context_exclude_paths: Option<Vec<String>>, // 额外排除路径
    pub context7_api_key: Option<String>,     // Context7 API密钥 (可选，免费使用时可为空)
    pub skill_python_path: Option<String>,    // Skill Python 路径（可选，默认走 PATH）

    // UI/UX Pro Max 配置
    /// 默认语言（"zh" | "en"）
    pub uiux_default_lang: Option<String>,
    /// 默认输出格式（"json" | "text"）
    pub uiux_output_format: Option<String>,
    /// 最大结果数上限（默认 10）
    pub uiux_max_results_cap: Option<u32>,
    /// 是否启用 UI 提示词美化（默认 true）
    pub uiux_beautify_enabled: Option<bool>,
    /// 知识检索后端："auto"（fast-context 可用则优先，默认）| "fast_context"（强制）| "local"（仅本地，便于 A/B 对比）
    pub uiux_knowledge_backend: Option<String>,

    // 图标工坊配置
    /// 默认保存路径（相对于项目根目录，如 "assets/icons"）
    pub icon_default_save_path: Option<String>,
    /// 默认保存格式: "svg" | "png" | "both"
    pub icon_default_format: Option<String>,
    /// PNG 尺寸（像素），默认 64
    pub icon_default_png_size: Option<u32>,
    /// 缓存过期时间（分钟），默认 30
    pub icon_cache_expiry_minutes: Option<u64>,

    // Tavily AI 搜索配置
    /// Tavily API 密钥（必填，免费计划每月 1000 信用点）
    pub tavily_api_key: Option<String>,
}

// 自定义prompt结构
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomPrompt {
    pub id: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default = "default_prompt_type")]
    pub r#type: String, // "normal" | "conditional"
    // 条件性prompt专用字段
    pub condition_text: Option<String>, // 条件描述文本
    pub template_true: Option<String>,  // 开关为true时的模板
    pub template_false: Option<String>, // 开关为false时的模板
    #[serde(default = "default_prompt_state")]
    pub current_state: bool, // 当前开关状态（原default_state）
    #[serde(default = "default_context_scope")]
    pub context_scope: String, // 条件上下文作用域：turn(本轮)、memory(长期记忆)、rule(项目规则)
    /// 关联的 MCP 工具 ID（仅对 conditional 类型有效）
    /// 当此字段有值时，该 prompt 的可用性取决于对应 MCP 工具的启用状态
    #[serde(default)]
    pub linked_mcp_tool: Option<String>,
}

// 自定义prompt配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomPromptConfig {
    #[serde(default = "default_custom_prompts")]
    pub prompts: Vec<CustomPrompt>,
    #[serde(default = "default_custom_prompt_enabled")]
    pub enabled: bool,
    #[serde(default = "default_custom_prompt_max_prompts")]
    pub max_prompts: u32,
}

// 快捷键配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShortcutConfig {
    #[serde(default = "default_shortcuts")]
    pub shortcuts: HashMap<String, ShortcutBinding>,
}

// 快捷键绑定
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub action: String, // "submit", "exit", "custom"
    pub key_combination: ShortcutKey,
    pub enabled: bool,
    pub scope: String, // "global", "popup", "input"
}

// 快捷键组合
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ShortcutKey {
    pub key: String, // 主键，如 "Enter", "Q", "F4"
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool, // macOS的Cmd键
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TelegramConfig {
    #[serde(default = "default_telegram_enabled")]
    pub enabled: bool, // 是否启用Telegram Bot
    #[serde(default = "default_telegram_bot_token")]
    pub bot_token: String, // Bot Token
    #[serde(default = "default_telegram_chat_id")]
    pub chat_id: String, // Chat ID
    #[serde(default = "default_telegram_hide_frontend_popup")]
    pub hide_frontend_popup: bool, // 是否隐藏前端弹窗，仅使用Telegram交互
    #[serde(default = "default_telegram_api_base_url")]
    pub api_base_url: String, // Telegram API基础URL
}

/// 代理配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProxyConfig {
    /// 是否启用自动检测代理
    #[serde(default = "default_proxy_auto_detect")]
    pub auto_detect: bool,

    /// 是否启用代理（手动模式）
    #[serde(default = "default_proxy_enabled")]
    pub enabled: bool,

    /// 代理类型: "http" 或 "socks5"
    #[serde(default = "default_proxy_type")]
    pub proxy_type: String,

    /// 代理主机地址
    #[serde(default = "default_proxy_host")]
    pub host: String,

    /// 代理端口
    #[serde(default = "default_proxy_port")]
    pub port: u16,

    /// 仅在中国大陆地区使用代理
    #[serde(default = "default_proxy_only_for_cn")]
    pub only_for_cn: bool,
}

#[derive(Debug)]
pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub response_channel: Mutex<Option<tokio::sync::oneshot::Sender<String>>>,
    // 防误触退出机制
    pub exit_attempt_count: Mutex<u32>,
    pub last_exit_attempt: Mutex<Option<std::time::Instant>>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui_config: default_ui_config(),
            audio_config: default_audio_config(),
            reply_config: default_reply_config(),
            mcp_config: default_mcp_config(),
            telegram_config: default_telegram_config(),
            custom_prompt_config: default_custom_prompt_config(),
            shortcut_config: default_shortcut_config(),
            proxy_config: default_proxy_config(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Mutex::new(AppConfig::default()),
            response_channel: Mutex::new(None),
            exit_attempt_count: Mutex::new(0),
            last_exit_attempt: Mutex::new(None),
        }
    }
}

// 默认值函数
pub fn default_ui_config() -> UiConfig {
    UiConfig {
        theme: default_theme(),
        font_config: default_font_config(),
        window_config: default_window_config(),
        always_on_top: default_always_on_top(),
        hljs_theme: default_hljs_theme(),
    }
}

pub fn default_audio_config() -> AudioConfig {
    AudioConfig {
        notification_enabled: default_audio_notification_enabled(),
        custom_url: default_audio_url(),
    }
}

pub fn default_mcp_config() -> McpConfig {
    McpConfig {
        tools: default_mcp_tools(),
        acemcp_base_url: None,
        acemcp_token: None,
        acemcp_batch_size: None,
        acemcp_max_lines_per_blob: None,
        acemcp_text_extensions: None,
        acemcp_exclude_patterns: None,
        acemcp_watch_debounce_ms: None, // 未设置时走 watcher 默认 30000ms (30秒)
        acemcp_watch_max_wait_ms: None, // 未设置时走 watcher 默认 300000ms (5分钟)
        acemcp_auto_index_enabled: None, // 默认启用（未设置时视为 true）
        acemcp_watched_projects: None,  // 默认没有持久监听项目
        acemcp_index_nested_projects: None, // 自动索引嵌套项目（默认启用）
        // 代理配置默认值
        acemcp_proxy_enabled: None,
        acemcp_proxy_host: None,
        acemcp_proxy_port: None,
        acemcp_proxy_type: None,
        acemcp_proxy_username: None,
        acemcp_proxy_password: None,
        // Sou 多后端默认配置
        sou_default_backend: Some("auto".to_string()),
        sou_auto_order: Some(vec!["ace".to_string(), "fast_context".to_string()]),
        sou_include_backend_headers: Some(true),
        sou_include_failed_backend_errors: Some(true),
        // Fast Context 默认配置：协议与本地命令执行已迁移为 Rust 原生实现
        fast_context_command: Some("node".to_string()),
        fast_context_script_path: None,
        fast_context_api_key: None,
        fast_context_tree_depth: Some(3),
        // 中文说明：与官方 Fast Context 最多四轮检索的公开行为保持一致。
        fast_context_max_turns: Some(4),
        fast_context_max_results: Some(10),
        fast_context_max_commands: Some(8),
        fast_context_timeout_ms: Some(30000),
        fast_context_exclude_paths: Some(vec![
            "node_modules".to_string(),
            ".git".to_string(),
            "dist".to_string(),
            "build".to_string(),
            "target".to_string(),
        ]),
        context7_api_key: None,
        skill_python_path: None,
        // UI/UX Pro Max 默认配置
        uiux_default_lang: Some("zh".to_string()),
        uiux_output_format: Some("json".to_string()),
        uiux_max_results_cap: Some(10),
        uiux_beautify_enabled: Some(true),
        uiux_knowledge_backend: Some("auto".to_string()),
        // 图标工坊配置默认值
        icon_default_save_path: None,    // 使用默认 "assets/icons"
        icon_default_format: None,       // 默认 SVG
        icon_default_png_size: None,     // 默认 64px
        icon_cache_expiry_minutes: None, // 默认 30 分钟
        // Tavily AI 搜索配置
        tavily_api_key: None, // 用户需配置 API Key
    }
}

pub fn default_telegram_config() -> TelegramConfig {
    TelegramConfig {
        enabled: default_telegram_enabled(),
        bot_token: default_telegram_bot_token(),
        chat_id: default_telegram_chat_id(),
        hide_frontend_popup: default_telegram_hide_frontend_popup(),
        api_base_url: default_telegram_api_base_url(),
    }
}

pub fn default_custom_prompt_config() -> CustomPromptConfig {
    CustomPromptConfig {
        prompts: default_custom_prompts(),
        enabled: default_custom_prompt_enabled(),
        max_prompts: default_custom_prompt_max_prompts(),
    }
}

pub fn default_always_on_top() -> bool {
    window::DEFAULT_ALWAYS_ON_TOP
}

pub fn default_hljs_theme() -> String {
    "auto".to_string()
}

pub fn default_audio_notification_enabled() -> bool {
    audio::DEFAULT_NOTIFICATION_ENABLED
}

pub fn default_theme() -> String {
    theme::DEFAULT.to_string()
}

pub fn default_audio_url() -> String {
    audio::DEFAULT_URL.to_string()
}

pub fn default_window_config() -> WindowConfig {
    WindowConfig {
        auto_resize: window::DEFAULT_AUTO_RESIZE,
        max_width: window::MAX_WIDTH,
        max_height: window::MAX_HEIGHT,
        min_width: window::MIN_WIDTH,
        min_height: window::MIN_HEIGHT,
        fixed: window::DEFAULT_FIXED_MODE,
        fixed_width: window::DEFAULT_WIDTH,
        fixed_height: window::DEFAULT_HEIGHT,
        free_width: window::DEFAULT_WIDTH,
        free_height: window::DEFAULT_HEIGHT,
    }
}

pub fn default_reply_config() -> ReplyConfig {
    ReplyConfig {
        enable_continue_reply: mcp::DEFAULT_CONTINUE_REPLY_ENABLED,
        auto_continue_threshold: mcp::DEFAULT_AUTO_CONTINUE_THRESHOLD,
        continue_prompt: mcp::DEFAULT_CONTINUE_PROMPT.to_string(),
    }
}

pub fn default_auto_resize() -> bool {
    true
}

pub fn default_max_width() -> f64 {
    window::MAX_WIDTH
}

pub fn default_max_height() -> f64 {
    window::MAX_HEIGHT
}

pub fn default_min_width() -> f64 {
    window::MIN_WIDTH
}

pub fn default_min_height() -> f64 {
    window::MIN_HEIGHT
}

pub fn default_enable_continue_reply() -> bool {
    mcp::DEFAULT_CONTINUE_REPLY_ENABLED
}

pub fn default_auto_continue_threshold() -> u32 {
    mcp::DEFAULT_AUTO_CONTINUE_THRESHOLD
}

pub fn default_continue_prompt() -> String {
    mcp::DEFAULT_CONTINUE_PROMPT.to_string()
}

pub fn default_mcp_tools() -> HashMap<String, bool> {
    let mut tools = HashMap::new();
    tools.insert(mcp::TOOL_ZHI.to_string(), true); // 三术工具默认启用（核心工具，不可禁用）
    tools.insert(mcp::TOOL_JI.to_string(), true); // 记忆管理工具默认启用（核心功能，不依赖外部配置，开箱即用）
    tools.insert(mcp::TOOL_SOU.to_string(), false); // 代码搜索工具默认关闭（依赖第三方 acemcp 服务，需要用户配置 token 和 URL）
    tools.insert(mcp::TOOL_CONTEXT7.to_string(), true); // Context7 文档查询工具默认启用（支持免费使用，无需配置即可使用）
    tools.insert(mcp::TOOL_UIUX.to_string(), true); // UI/UX 工具默认启用（内置技能）
    tools.insert(mcp::TOOL_ENHANCE.to_string(), false); // 提示词增强工具默认关闭（依赖 acemcp 配置）
    tools.insert(mcp::TOOL_TAVILY.to_string(), true); // Tavily AI 搜索工具默认启用（免费额度，需配置 API Key）
    tools.insert(mcp::TOOL_PLAN.to_string(), true); // 开发计划跟踪为纯本地能力，默认启用
    tools
}

pub fn default_window_width() -> f64 {
    window::DEFAULT_WIDTH
}

pub fn default_window_height() -> f64 {
    window::DEFAULT_HEIGHT
}

pub fn default_window_fixed() -> bool {
    window::DEFAULT_FIXED_MODE
}

pub fn default_fixed_width() -> f64 {
    window::DEFAULT_WIDTH
}

pub fn default_fixed_height() -> f64 {
    window::DEFAULT_HEIGHT
}

pub fn default_free_width() -> f64 {
    window::DEFAULT_WIDTH
}

pub fn default_free_height() -> f64 {
    window::DEFAULT_HEIGHT
}

pub fn default_telegram_enabled() -> bool {
    telegram::DEFAULT_ENABLED
}

pub fn default_telegram_bot_token() -> String {
    telegram::DEFAULT_BOT_TOKEN.to_string()
}

pub fn default_telegram_chat_id() -> String {
    telegram::DEFAULT_CHAT_ID.to_string()
}

pub fn default_telegram_hide_frontend_popup() -> bool {
    telegram::DEFAULT_HIDE_FRONTEND_POPUP
}

pub fn default_telegram_api_base_url() -> String {
    telegram::API_BASE_URL.to_string()
}

impl WindowConfig {
    // 获取当前模式的宽度
    pub fn current_width(&self) -> f64 {
        if self.fixed {
            self.fixed_width
        } else {
            self.free_width
        }
    }

    // 获取当前模式的高度
    pub fn current_height(&self) -> f64 {
        if self.fixed {
            self.fixed_height
        } else {
            self.free_height
        }
    }

    // 更新当前模式的尺寸
    pub fn update_current_size(&mut self, width: f64, height: f64) {
        if self.fixed {
            self.fixed_width = width;
            self.fixed_height = height;
        } else {
            self.free_width = width;
            self.free_height = height;
        }
    }
}

// 字体配置默认值函数
pub fn default_font_config() -> FontConfig {
    FontConfig {
        font_family: default_font_family(),
        font_size: default_font_size(),
        custom_font_family: default_custom_font_family(),
    }
}

pub fn default_font_family() -> String {
    font::DEFAULT_FONT_FAMILY.to_string()
}

pub fn default_font_size() -> String {
    font::DEFAULT_FONT_SIZE.to_string()
}

pub fn default_custom_font_family() -> String {
    font::DEFAULT_CUSTOM_FONT_FAMILY.to_string()
}

pub fn default_prompt_type() -> String {
    "normal".to_string()
}

pub fn default_prompt_state() -> bool {
    false
}

// 自定义prompt默认值函数
pub fn default_custom_prompts() -> Vec<CustomPrompt> {
    // 中文说明：内置普通快捷模板统一使用中文名称，降低弹窗中的语言切换成本。
    vec![
        CustomPrompt {
            id: "default_1".to_string(),
            name: "✅完成".to_string(),
            content: "结束当前对话".to_string(),
            description: Some("请求AI结束工作".to_string()),
            sort_order: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_2".to_string(),
            name: "🧹清空".to_string(),
            content: "".to_string(),
            description: Some("清空输入框内容".to_string()),
            sort_order: 2,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_3".to_string(),
            name: "✨新需求".to_string(),
            content: "ok，完美，新的需求or问题，".to_string(),
            description: Some("准备新的需求or问题".to_string()),
            sort_order: 3,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_4".to_string(),
            name: "🧠记住".to_string(),
            content: "请记住，".to_string(),
            description: Some("三术的另一个工具，请记住".to_string()),
            sort_order: 4,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_5".to_string(),
            name: "📝总结并重启".to_string(),
            content: "本次对话的上下文已经太长了，我打算关掉并重新开一个新的会话。你有什么想对你的继任者说的，以便它能更好的理解你当前的工作并顺利继续？".to_string(),
            description: Some("总结-开新会话".to_string()),
            sort_order: 5,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_6".to_string(),
            name: "🔍审查与规划".to_string(),
            content: "请执行以下项目进度检查和规划任务：\n\n1. **项目进度分析**：\n   - 查看当前代码库状态，分析已完成的功能模块\n   - 识别已完成、进行中和待开始的功能点\n\n2. **里程碑确定**：\n   - 基于当前进度和剩余工作量，定义清晰的里程碑节点\n   - 为每个里程碑设定具体的完成标准和时间预期\n   - 优先考虑核心任务管理功能的里程碑\n\n3. **文档更新**（注意：仅更新现有文档，不创建新文档）：\n   - 更新项目规划文档中的进度状态\n   - 修正任何与实际实现不符的技术方案描述\n   - 确保文档反映当前的技术栈和架构决策\n\n4. **下一步工作规划**：\n   - 基于用户偏好（系统化开发方法、前端优先、分步骤反馈）制定具体的下一阶段工作计划\n   - 识别关键路径上的阻塞点和依赖关系\n   - 提供3-5个具体的下一步行动项，按优先级排序\n\n5. **反馈收集**：\n   - 在完成分析后，使用三术工具收集用户对进度评估和下一步计划的反馈\n   - 提供多个可选的发展方向供用户选择".to_string(),
            description: Some("项目进度检查和规划任务".to_string()),
            sort_order: 6,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_7".to_string(),
            name: "是否生成总结性Markdown文档".to_string(),
            content: "".to_string(),
            description: Some("是否生成总结性Markdown文档".to_string()),
            sort_order: 7,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否生成总结性Markdown文档".to_string()),
            template_true: Some("✔️请记住，帮我生成总结性Markdown文档".to_string()),
            template_false: Some("❌请记住，不要生成总结性Markdown文档".to_string()),
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_8".to_string(),
            name: "是否生成测试脚本".to_string(),
            content: "".to_string(),
            description: Some("是否生成测试脚本".to_string()),
            sort_order: 8,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否生成测试脚本".to_string()),
            template_true: Some("✔️请记住，帮我生成测试脚本".to_string()),
            template_false: Some("❌请记住，不要生成测试脚本".to_string()),
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_9".to_string(),
            name: "是否主动编译".to_string(),
            content: "".to_string(),
            description: Some("是否主动编译".to_string()),
            sort_order: 9,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否主动编译".to_string()),
            template_true: Some("✔️请记住，帮我编译".to_string()),
            template_false: Some("❌请记住，不要编译，用户自己编译".to_string()),
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        CustomPrompt {
            id: "default_10".to_string(),
            name: "是否主动运行".to_string(),
            content: "".to_string(),
            description: Some("是否主动运行".to_string()),
            sort_order: 10,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否主动运行".to_string()),
            template_true: Some("✔️请记住，帮我运行".to_string()),
            template_false: Some("❌请记住，不要运行，用户自己运行".to_string()),
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        // MCP 功能性工具联动 prompt
        CustomPrompt {
            id: "default_11".to_string(),
            name: "是否启用代码搜索工具".to_string(),
            content: "".to_string(),
            description: Some("控制 sou 代码语义搜索工具的使用".to_string()),
            sort_order: 11,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否启用代码搜索工具".to_string()),
            template_true: Some("✔️请记住，使用 sou 进行代码语义搜索，根据结果到指定位置查看更多上下文".to_string()),
            template_false: Some("".to_string()),
            current_state: false, // 默认关闭（与 TOOL_SOU 默认状态保持一致）
            context_scope: "turn".to_string(),
            linked_mcp_tool: Some("sou".to_string()), // 关联到 sou MCP 工具
        },
        CustomPrompt {
            id: "default_12".to_string(),
            name: "是否启用框架文档查询".to_string(),
            content: "".to_string(),
            description: Some("控制 context7 框架文档查询工具的使用".to_string()),
            sort_order: 12,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否启用框架文档查询".to_string()),
            template_true: Some("✔️请记住，使用 context7 查询框架/库的最新官方文档和 API 用法".to_string()),
            template_false: Some("".to_string()),
            current_state: true, // 默认开启（与 TOOL_CONTEXT7 默认状态保持一致）
            context_scope: "turn".to_string(),
            linked_mcp_tool: Some("context7".to_string()), // 关联到 context7 MCP 工具
        },
        CustomPrompt {
            id: "default_13".to_string(),
            name: "是否通过三术确认关键节点".to_string(),
            content: "".to_string(),
            description: Some("提醒 AI 通过 zhi 展示关键提问、方案确认和完成确认".to_string()),
            sort_order: 13,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否通过三术确认关键节点".to_string()),
            template_true: Some("✔️请记住，所有关键提问、方案确认和完成确认都必须通过 zhi 展示".to_string()),
            template_false: Some("".to_string()),
            current_state: true, // zhi 是核心工具，默认提醒开启
            context_scope: "turn".to_string(),
            linked_mcp_tool: Some("zhi".to_string()),
        },
        CustomPrompt {
            id: "default_14".to_string(),
            name: "是否优先使用 UI/UX 单工具".to_string(),
            content: "".to_string(),
            description: Some("提醒 AI 涉及页面美化、UI 描述、设计系统或审查时优先使用 uiux".to_string()),
            sort_order: 14,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否优先使用 UI/UX 单工具".to_string()),
            template_true: Some("✔️请记住，涉及页面美化、UI 描述、设计系统或 UI 审查时，优先使用单一 uiux 工具，并参考它返回的 prompt、uiux_hits 与 project_context".to_string()),
            template_false: Some("".to_string()),
            current_state: true, // uiux 是当前主链路，默认提醒开启
            context_scope: "turn".to_string(),
            linked_mcp_tool: Some("uiux".to_string()),
        },
        CustomPrompt {
            id: "default_15".to_string(),
            name: "是否启用 AI 搜索".to_string(),
            content: "".to_string(),
            description: Some("提醒 AI 需要实时信息时使用 tavily 进行搜索".to_string()),
            sort_order: 15,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否启用 AI 搜索".to_string()),
            template_true: Some("✔️请记住，需要实时搜索信息时使用 tavily 工具进行 AI 搜索，获取最新准确数据".to_string()),
            template_false: Some("".to_string()),
            current_state: true, // 默认开启（与 TOOL_TAVILY 默认状态保持一致）
            context_scope: "turn".to_string(),
            linked_mcp_tool: Some("tavily".to_string()), // 关联到 tavily MCP 工具
        },
        // 中文说明：仅约束本次需求的剩余任务，并为真正无法继续的阻塞保留确认出口。
        CustomPrompt {
            id: "default_16".to_string(),
            name: "🚀自主完成".to_string(),
            content: concat!(
                "【执行模式：自主完成】\n",
                "范围：本次需求的全部剩余任务。\n",
                "规则：\n",
                "1. 除非出现因信息缺失而无法安全继续的阻塞，否则不再调用 zhi 进行中途确认。\n",
                "2. 基于已检索的现有代码、明确需求与项目规范，自主选择整体最优且最小必要的实现，持续执行至全部完成。\n",
                "3. 记录关键决策及理由；全部完成后统一通过 zhi 汇报实现、决策依据、验证结果与剩余风险，并请求最终确认。"
            )
            .to_string(),
            description: Some("减少中途确认，自主完成本次需求后统一汇报".to_string()),
            sort_order: 16,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        // 中文说明：项目审查模板要求从重度用户视角出发，并用真实代码证据约束结论。
        CustomPrompt {
            id: "default_17".to_string(),
            name: "💡项目改进审查".to_string(),
            content: concat!(
                "【角色】作为本项目的高频重度用户和长期使用者。\n",
                "【任务】基于实际检索与读取的源代码，审查仍需改进的功能与体验。\n",
                "【输出要求】\n",
                "1. 按用户价值、影响范围与实施成本排序。\n",
                "2. 每项提供代码证据、现状问题、改进方案、预期收益、风险与验收标准。\n",
                "3. 覆盖使用效率、易用性、稳定性和可维护性。\n",
                "4. 仅提出可落地、可验证且符合 KISS、YAGNI、SOLID 的建议；禁止臆测。"
            )
            .to_string(),
            description: Some("从重度用户视角审查项目改进空间".to_string()),
            sort_order: 17,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "normal".to_string(),
            condition_text: None,
            template_true: None,
            template_false: None,
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: None,
        },
        // 中文说明：计划工具默认可用，但只有用户主动开启本项时才要求 AI 维护计划。
        CustomPrompt {
            id: "default_18".to_string(),
            name: "是否启用计划跟踪".to_string(),
            content: "".to_string(),
            description: Some("控制 AI 是否通过 plan 工具维护当前工作区执行计划".to_string()),
            sort_order: 18,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            r#type: "conditional".to_string(),
            condition_text: Some("是否启用计划跟踪".to_string()),
            template_true: Some(
                "请使用 plan 工具跟踪当前工作区的开发计划：开始开发前用 set 提交完整计划；开始某一步时用 update 标记 in_progress；完成后用 update 标记 completed；结束前用 get 核对计划状态。"
                    .to_string(),
            ),
            template_false: Some("".to_string()),
            current_state: false,
            context_scope: "turn".to_string(),
            linked_mcp_tool: Some("plan".to_string()),
        },
    ]
}

pub fn default_custom_prompt_enabled() -> bool {
    true
}

fn default_context_scope() -> String {
    "turn".to_string()
}

pub fn default_custom_prompt_max_prompts() -> u32 {
    50
}

// 快捷键默认值函数
pub fn default_shortcut_config() -> ShortcutConfig {
    ShortcutConfig {
        shortcuts: default_shortcuts(),
    }
}

pub fn default_shortcuts() -> HashMap<String, ShortcutBinding> {
    let mut shortcuts = HashMap::new();

    // 快速发送快捷键
    shortcuts.insert(
        "quick_submit".to_string(),
        ShortcutBinding {
            id: "quick_submit".to_string(),
            name: "快速发送".to_string(),
            description: "快速提交当前输入内容".to_string(),
            action: "submit".to_string(),
            key_combination: ShortcutKey {
                key: "Enter".to_string(),
                ctrl: true,
                alt: false,
                shift: false,
                meta: false,
            },
            enabled: true,
            scope: "popup".to_string(),
        },
    );

    // 增强快捷键
    shortcuts.insert(
        "enhance".to_string(),
        ShortcutBinding {
            id: "enhance".to_string(),
            name: "增强".to_string(),
            description: "增强当前输入内容".to_string(),
            action: "enhance".to_string(),
            key_combination: ShortcutKey {
                key: "Enter".to_string(),
                ctrl: true,
                alt: false,
                shift: true,
                meta: false,
            },
            enabled: true,
            scope: "popup".to_string(),
        },
    );

    // 继续快捷键
    shortcuts.insert(
        "continue".to_string(),
        ShortcutBinding {
            id: "continue".to_string(),
            name: "继续".to_string(),
            description: "继续对话".to_string(),
            action: "continue".to_string(),
            key_combination: ShortcutKey {
                key: "Enter".to_string(),
                ctrl: false,
                alt: true,
                shift: false,
                meta: false,
            },
            enabled: true,
            scope: "popup".to_string(),
        },
    );

    shortcuts
}

// 代理配置默认值函数
pub fn default_proxy_config() -> ProxyConfig {
    ProxyConfig {
        auto_detect: default_proxy_auto_detect(),
        enabled: default_proxy_enabled(),
        proxy_type: default_proxy_type(),
        host: default_proxy_host(),
        port: default_proxy_port(),
        only_for_cn: default_proxy_only_for_cn(),
    }
}

pub fn default_proxy_auto_detect() -> bool {
    true // 默认启用自动检测
}

pub fn default_proxy_enabled() -> bool {
    false // 默认不启用手动代理
}

pub fn default_proxy_type() -> String {
    "http".to_string() // 默认使用HTTP代理
}

pub fn default_proxy_host() -> String {
    "127.0.0.1".to_string() // 默认本地代理
}

pub fn default_proxy_port() -> u16 {
    7890 // 默认Clash混合代理端口
}

pub fn default_proxy_only_for_cn() -> bool {
    true // 默认仅在中国大陆地区使用代理
}
