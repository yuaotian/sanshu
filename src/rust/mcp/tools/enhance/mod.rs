// 提示词增强模块
// 调用 Augment chat-stream API 将口语化提示词转换为结构化专业提示词

pub mod types;
pub mod core;
pub mod history;
pub mod commands;
pub mod mcp;

// 重新导出工具以便访问
pub use mcp::EnhanceTool;
pub use types::*;
pub use core::*;
pub use history::ChatHistoryManager;
