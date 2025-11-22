use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
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
    /// 索引失败
    Failed,
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
    /// 按目录聚合的统计信息（目录路径 -> (已索引, 待处理)）
    pub directory_stats: HashMap<String, (usize, usize)>,
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
            directory_stats: HashMap::new(),
        }
    }
}

/// 所有项目的索引状态集合
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectsIndexStatus {
    /// 项目路径 -> 索引状态
    pub projects: HashMap<String, ProjectIndexStatus>,
}
