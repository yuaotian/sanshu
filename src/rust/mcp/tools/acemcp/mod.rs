// Acemcp工具模块
// 用于代码库索引和语义搜索的MCP工具

pub mod mcp;
pub mod types;
pub mod commands;
pub mod watcher;

// 重新导出工具以便访问
pub use mcp::AcemcpTool;
pub use watcher::get_watcher_manager;
