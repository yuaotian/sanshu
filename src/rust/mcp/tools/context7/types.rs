use serde::{Deserialize, Serialize};

/// Context7 查询请求参数
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Context7Request {
    /// 库标识符，格式: owner/repo (例如: "vercel/next.js", "facebook/react")
    #[schemars(description = "库标识符，格式: owner/repo (例如: vercel/next.js, facebook/react)")]
    pub library: String,
    /// 查询主题 (可选，例如: "routing", "authentication")
    #[schemars(description = "查询主题 (可选，例如: routing, authentication)")]
    #[serde(default)]
    pub topic: Option<String>,
    /// 版本号 (可选，例如: "v15.1.8")
    #[schemars(description = "版本号 (可选，例如: v15.1.8)")]
    #[serde(default)]
    pub version: Option<String>,
    /// 分页页码 (可选，默认1，最大10)
    #[schemars(description = "分页页码 (可选，默认1，最大10)")]
    #[serde(default)]
    pub page: Option<u32>,
}

/// Context7 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context7Config {
    /// API 密钥 (可选，免费使用时可为空)
    pub api_key: Option<String>,
    /// API 基础 URL
    pub base_url: String,
}

impl Default for Context7Config {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: "https://context7.com/api/v2".to_string(),
        }
    }
}

/// Context7 API 响应结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context7Response {
    /// 文档片段列表
    pub snippets: Vec<DocumentSnippet>,
    /// 分页信息
    pub pagination: Option<PaginationInfo>,
}

/// 文档片段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSnippet {
    /// 片段内容 (Markdown 格式)
    pub content: String,
    /// 片段标题
    pub title: Option<String>,
    /// 相关性分数
    pub score: Option<f64>,
}

/// 分页信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationInfo {
    /// 当前页码
    pub current_page: u32,
    /// 总页数
    pub total_pages: u32,
    /// 是否有下一页
    pub has_next: bool,
}

/// 测试连接响应
#[derive(Debug, Serialize, Deserialize)]
pub struct TestConnectionResponse {
    /// 是否成功
    pub success: bool,
    /// 提示消息
    pub message: String,
    /// 文档预览 (可选)
    pub preview: Option<String>,
}

/// 库重定向响应（301）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryRedirectResponse {
    pub error: String,
    pub message: String,
    #[serde(rename = "redirectUrl")]
    pub redirect_url: String,
}

/// 库搜索响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// 搜索结果列表
    pub results: Vec<SearchResult>,
}

/// 搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// 库标识符 (格式: /owner/repo)
    pub id: String,
    /// 库名称
    pub title: Option<String>,
    /// 库描述
    pub description: Option<String>,
    /// GitHub stars 数量
    pub stars: Option<u64>,
    /// 信任分数 (0-10)
    #[serde(rename = "trustScore")]
    pub trust_score: Option<f64>,
    /// 基准测试分数
    #[serde(rename = "benchmarkScore")]
    pub benchmark_score: Option<f64>,
}

