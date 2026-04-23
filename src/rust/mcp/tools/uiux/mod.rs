// UI/UX MCP 工具模块
// 当前主链路已收敛为单一 uiux 工具：sou-first + 本地 markdown 降级。

pub mod localize;
pub mod markdown_search;
pub mod mcp;
pub mod response;
pub mod types;

pub use mcp::UiuxTool;
