//! 方案记录与反馈处理工具模块
//!
//! 提供方案摘要记录、候选项整理与反馈结果解析能力

pub mod mcp;
pub mod zhi_history;
pub mod commands;

// 重新导出主要类型和功能
pub use mcp::InteractionTool;
pub use zhi_history::{ZhiHistoryEntry, ZhiHistoryManager};
