//! 记忆管理工具模块
//!
//! 提供全局记忆管理功能，用于存储和管理重要的开发规范、用户偏好和最佳实践
//!
//! ## 模块结构
//! - `types` - 数据类型定义（MemoryEntry, MemoryStore, MemoryConfig）
//! - `similarity` - 文本相似度算法
//! - `dedup` - 去重检测器
//! - `manager` - 核心管理器
//! - `mcp` - MCP 接口

pub mod types;
pub mod similarity;
pub mod dedup;
pub mod manager;
pub mod mcp;

pub use manager::MemoryManager;
pub use types::{MemoryEntry, MemoryCategory, MemoryMetadata, MemoryStore, MemoryConfig};
pub use mcp::MemoryTool;
pub use similarity::TextSimilarity;
pub use dedup::{MemoryDeduplicator, DuplicateInfo, DedupResult};
