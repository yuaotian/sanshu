// UI/UX MCP 工具模块
// 当前主链路已收敛为单一 uiux 工具：本地 BM25/BGE 混合检索 + 显式 fast-context A/B 诊断。

pub mod knowledge_base;
mod lexicon;
pub mod localize;
pub mod mcp;
pub mod model_manager;
pub mod response;
pub mod semantic_search;
pub mod structured_search;
pub mod types;

pub use mcp::UiuxTool;
