use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Acemcp搜索请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcemcpRequest {
    /// 项目根目录的绝对路径
    pub project_root_path: String,
    /// 用于查找相关代码上下文的自然语言搜索查询
    pub query: String,
}

/// Acemcp配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcemcpConfig {
    /// API端点URL
    pub base_url: Option<String>,
    /// 认证令牌
    pub token: Option<String>,
    /// 每批上传的文件数量
    pub batch_size: Option<u32>,
    /// 大文件分割前的最大行数
    pub max_lines_per_blob: Option<u32>,
    /// 要索引的文件扩展名列表
    pub text_extensions: Option<Vec<String>>,
    /// 要排除的模式列表
    pub exclude_patterns: Option<Vec<String>>,
    /// 搜索时的智能等待配置（秒）
    /// 当检测到索引正在进行时，随机等待 [min, max] 秒后再执行搜索
    /// 默认值：Some((1, 5))，设为 None 则禁用智能等待
    pub smart_wait_range: Option<(u64, u64)>,
    // 代理配置
    /// 是否启用代理
    pub proxy_enabled: Option<bool>,
    /// 代理主机地址
    pub proxy_host: Option<String>,
    /// 代理端口
    pub proxy_port: Option<u16>,
    /// 代理类型: "http" | "https" | "socks5"
    pub proxy_type: Option<String>,
    /// 代理用户名（可选）
    pub proxy_username: Option<String>,
    /// 代理密码（可选）
    pub proxy_password: Option<String>,
}

/// 索引状态枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum IndexStatus {
    /// 空闲状态（未开始索引）
    Idle,
    /// 正在索引中
    Indexing,
    /// 索引成功完成
    Synced,
    /// 上传中断但保留断点，等待后台恢复
    Paused,
    /// 索引失败
    Failed,
}

/// 项目索引范围的风险等级。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectScopeRiskLevel {
    /// 文件系统根目录或用户主目录等关键路径。
    CriticalPath,
    /// 候选文件数量或体积超过有界预检阈值。
    ExcessiveScale,
}

/// 项目索引范围风险诊断，供后端持久化并由 GUI 明确展示原因。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectScopeRisk {
    pub level: ProjectScopeRiskLevel,
    pub reason_code: String,
    pub reason: String,
    pub scanned_entries: usize,
    pub candidate_files: usize,
    pub candidate_bytes: u64,
    pub project_markers: Vec<String>,
    pub requires_secondary_confirmation: bool,
    pub detected_at: DateTime<Utc>,
}

/// 项目索引状态信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIndexStatus {
    /// 项目根路径（规范化后）
    pub project_root: String,
    /// 当前索引状态
    pub status: IndexStatus,
    /// 索引进度百分比（0-100）
    pub progress: u8,
    /// 总文件数
    pub total_files: usize,
    /// 已索引文件数
    pub indexed_files: usize,
    /// 待处理文件数
    pub pending_files: usize,
    /// 失败文件数
    pub failed_files: usize,
    /// 最后成功索引时间
    pub last_success_time: Option<DateTime<Utc>>,
    /// 最后失败时间
    pub last_failure_time: Option<DateTime<Utc>>,
    /// 最后错误信息
    pub last_error: Option<String>,
    /// 最近一次认证失败所对应的索引空间签名
    /// 用于在用户更新 Token 后恢复自动索引
    pub last_failure_scope_hash: Option<String>,
    /// 当前项目最近一次成功索引所对应的索引空间签名
    /// 用于检测 base_url/token 变更后旧索引是否失效
    #[serde(default)]
    pub index_scope_hash: Option<String>,
    /// 当前索引是否与现有 ACE 配置不匹配
    #[serde(default)]
    pub is_stale: bool,
    /// 索引失效原因说明（用于前端展示）
    #[serde(default)]
    pub stale_reason: Option<String>,
    /// 按目录聚合的统计信息（目录路径 -> (已索引, 待处理)）
    pub directory_stats: HashMap<String, (usize, usize)>,
    /// 最近增量索引的文件列表（最多保留 5 个，相对路径）
    #[serde(default)]
    pub recent_indexed_files: Vec<String>,
    /// 当前后台任务 ID（从 index_jobs.json 校正）
    #[serde(default)]
    pub job_id: Option<String>,
    /// 当前任务总批次
    #[serde(default)]
    pub total_batches: usize,
    /// 当前任务已完成批次
    #[serde(default)]
    pub completed_batches: usize,
    /// 任务检查点最后更新时间
    #[serde(default)]
    pub job_updated_at: Option<DateTime<Utc>>,
    /// 当前项目是否因范围异常而暂停索引。
    #[serde(default)]
    pub scope_risk: Option<ProjectScopeRisk>,
}

impl Default for ProjectIndexStatus {
    fn default() -> Self {
        Self {
            project_root: String::new(),
            status: IndexStatus::Idle,
            progress: 0,
            total_files: 0,
            indexed_files: 0,
            pending_files: 0,
            failed_files: 0,
            last_success_time: None,
            last_failure_time: None,
            last_error: None,
            last_failure_scope_hash: None,
            index_scope_hash: None,
            is_stale: false,
            stale_reason: None,
            directory_stats: HashMap::new(),
            recent_indexed_files: Vec::new(),
            job_id: None,
            total_batches: 0,
            completed_batches: 0,
            job_updated_at: None,
            scope_risk: None,
        }
    }
}

/// 所有项目的索引状态集合
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectsIndexStatus {
    /// 项目路径 -> 索引状态
    pub projects: HashMap<String, ProjectIndexStatus>,
}

/// 单个文件的索引状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileIndexStatusKind {
    /// 文件已完成索引（所有 blob 均已上传并记录）
    Indexed,
    /// 文件已被纳入候选集合但尚未全部完成索引或需要重新上传
    Pending,
}

/// 文件索引状态信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndexStatus {
    /// 相对于项目根目录的文件路径，使用正斜杠(/)分隔
    pub path: String,
    /// 文件索引状态
    pub status: FileIndexStatusKind,
}

/// 项目内所有可索引文件的状态集合（用于前端构建项目结构树）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectFilesStatus {
    /// 项目根路径（规范化后）
    pub project_root: String,
    /// 文件状态列表
    pub files: Vec<FileIndexStatus>,
}

/// 嵌套项目信息（检测到的子目录中的独立 Git 仓库或子项目）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedProjectInfo {
    /// 子项目路径（相对于父项目根目录）
    pub relative_path: String,
    /// 子项目绝对路径
    pub absolute_path: String,
    /// 是否是独立的 Git 仓库（包含 .git 目录）
    pub is_git_repo: bool,
    /// 子项目的索引状态（如果存在）
    pub index_status: Option<ProjectIndexStatus>,
    /// 子项目包含的文件数量（粗略估计）
    pub file_count: usize,
}

/// 包含嵌套项目信息的项目状态（用于前端多项目展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectWithNestedStatus {
    /// 主项目（外层目录）的索引状态
    pub root_status: ProjectIndexStatus,
    /// 检测到的嵌套项目列表
    pub nested_projects: Vec<NestedProjectInfo>,
    /// 普通子目录列表（不含 .git）
    pub regular_directories: Vec<String>,
}

// ============ 代理测速相关类型 ============

/// 检测到的代理信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedProxy {
    /// 代理主机
    pub host: String,
    /// 代理端口
    pub port: u16,
    /// 代理类型: "http" | "socks5"
    pub proxy_type: String,
    /// 响应时间（毫秒），用于排序
    pub response_time_ms: Option<u64>,
}

/// 代理测速结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySpeedTestResult {
    /// 测试模式: "proxy" | "direct" | "compare"
    pub mode: String,
    /// 代理配置信息（仅代理模式有效）
    pub proxy_info: Option<DetectedProxy>,
    /// 测试指标列表
    pub metrics: Vec<SpeedTestMetric>,
    /// 测试时间戳
    pub timestamp: String,
    /// 总体推荐建议
    pub recommendation: String,
    /// 是否全部测试成功
    pub success: bool,
}

/// 搜索结果预览片段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultSnippet {
    /// 文件路径
    pub file_path: String,
    /// 匹配的代码片段（截断后）
    pub snippet: String,
    /// 片段在文件中的起始行号（可选）
    pub line_number: Option<u32>,
}

/// 搜索结果预览（用于测速结果展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultPreview {
    /// 总匹配数
    pub total_matches: usize,
    /// 预览片段（最多3条）
    pub snippets: Vec<SearchResultSnippet>,
    /// 原始响应长度（字符数）
    pub response_length: usize,
}

/// 单项测试指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedTestMetric {
    /// 指标名称（如 "网络延迟"、"单文件上传"、"语义搜索"）
    pub name: String,
    /// 指标类型: "ping" | "upload_single" | "search"
    pub metric_type: String,
    /// 代理模式耗时（毫秒）
    pub proxy_time_ms: Option<u64>,
    /// 直连模式耗时（毫秒）
    pub direct_time_ms: Option<u64>,
    /// 是否成功
    pub success: bool,
    /// 错误信息
    pub error: Option<String>,
    /// 搜索结果预览（仅 search 类型有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_result_preview: Option<SearchResultPreview>,
}

// ============== 测速进度反馈 ==============

/// 测速阶段状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SpeedTestStageStatus {
    /// 等待中
    Pending,
    /// 进行中
    Running,
    /// 已完成
    Completed,
    /// 失败
    Failed,
}

/// 测速进度事件（用于前端实时展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedTestProgress {
    /// 阶段编号 (0-5)
    pub stage: u8,
    /// 阶段名称（中文，如 "初始化"、"Ping测试"）
    pub stage_name: String,
    /// 总体进度百分比 (0-100)
    pub percentage: u8,
    /// 阶段状态
    pub status: SpeedTestStageStatus,
    /// 阶段详情（关键指标摘要，如 "avg=236ms, 3/3"）
    pub detail: Option<String>,
    /// 子步骤名称（可选，用于更细粒度的进度，如 "代理 Ping"）
    pub sub_step: Option<String>,
}
