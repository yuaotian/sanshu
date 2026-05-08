// Tavily AI 搜索工具类型定义

use serde::{Deserialize, Serialize};

/// Tavily MCP 请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyRequest {
    /// 搜索查询（search 时必填）
    #[serde(default)]
    pub query: Option<String>,
    /// 操作类型：search（默认）或 extract
    #[serde(default = "default_action")]
    pub action: String,
    /// 搜索深度："basic"（1信用）或 "advanced"（2信用）
    #[serde(default)]
    pub search_depth: Option<String>,
    /// 最大结果数（0-20，默认5）
    #[serde(default)]
    pub max_results: Option<u32>,
    /// 搜索类别："general"、"news" 或 "finance"
    #[serde(default)]
    pub topic: Option<String>,
    /// 时间范围过滤："day"、"week"、"month"、"year"
    #[serde(default)]
    pub time_range: Option<String>,
    /// 是否包含 AI 生成的回答：false / "basic" / "advanced"
    #[serde(default)]
    pub include_answer: Option<serde_json::Value>,
    /// 是否包含原始内容：false / "markdown" / "text"
    #[serde(default)]
    pub include_raw_content: Option<serde_json::Value>,
    /// 域名白名单
    #[serde(default)]
    pub include_domains: Option<Vec<String>>,
    /// 域名黑名单
    #[serde(default)]
    pub exclude_domains: Option<Vec<String>>,
    /// 提取 URL（extract 时必填，支持单个字符串或数组）
    #[serde(default)]
    pub urls: Option<serde_json::Value>,
    /// 提取深度："basic" 或 "advanced"
    #[serde(default)]
    pub extract_depth: Option<String>,
}

fn default_action() -> String {
    "search".to_string()
}

/// Tavily 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyConfig {
    /// API 密钥（必填）
    pub api_key: Option<String>,
    /// API 基础 URL
    pub base_url: String,
}

impl Default for TavilyConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: "https://api.tavily.com".to_string(),
        }
    }
}

// ============ Search API 响应结构 ============

/// Tavily Search API 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilySearchResponse {
    pub query: String,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub images: Vec<TavilyImage>,
    #[serde(default)]
    pub results: Vec<TavilySearchResult>,
    #[serde(default)]
    pub response_time: Option<f64>,
    #[serde(default)]
    pub request_id: Option<String>,
}

/// 搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilySearchResult {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub raw_content: Option<String>,
    #[serde(default)]
    pub favicon: Option<String>,
}

/// 图片项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyImage {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

// ============ Extract API 响应结构 ============

/// Tavily Extract API 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyExtractResponse {
    #[serde(default)]
    pub results: Vec<TavilyExtractResult>,
    #[serde(default)]
    pub failed_results: Vec<TavilyFailedResult>,
    #[serde(default)]
    pub response_time: Option<f64>,
    #[serde(default)]
    pub request_id: Option<String>,
}

/// 提取结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyExtractResult {
    pub url: String,
    #[serde(default)]
    pub raw_content: Option<String>,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub favicon: Option<String>,
}

/// 提取失败项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TavilyFailedResult {
    pub url: String,
    #[serde(default)]
    pub error: Option<String>,
}

// ============ 测试连接响应 ============

/// 测试连接响应
#[derive(Debug, Serialize, Deserialize)]
pub struct TavilyTestConnectionResponse {
    pub success: bool,
    pub message: String,
    /// 搜索结果预览（可选）
    pub preview: Option<String>,
}
