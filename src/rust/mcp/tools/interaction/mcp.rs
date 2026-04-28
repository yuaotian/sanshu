use anyhow::Result;
use rmcp::model::{ErrorData as McpError, CallToolResult};

use crate::mcp::{ZhiRequest, PopupRequest};
use crate::mcp::handlers::{create_tauri_popup, parse_mcp_response};
use crate::mcp::utils::{generate_request_id, popup_error};
use crate::mcp::utils::safe_truncate_clean;
use crate::{log_important, log_debug};

/// 代码审阅记录工具
///
/// 汇总审阅内容、候选处理项与结构化反馈
#[derive(Clone)]
pub struct InteractionTool;

impl InteractionTool {
    pub async fn zhi(
        request: ZhiRequest,
    ) -> Result<CallToolResult, McpError> {
        // 默认生成 request_id（MCP server 会优先使用其 call_id 注入到 zhi_with_request_id）
        let request_id = generate_request_id();
        Self::zhi_with_request_id(request, request_id).await
    }

    /// 带 request_id 的 zhi 调用入口
    ///
    /// 中文说明：用于将 MCP 分发层生成的 call_id 贯穿到 GUI 进程与响应，便于全链路日志关联。
    pub async fn zhi_with_request_id(
        request: ZhiRequest,
        request_id: String,
    ) -> Result<CallToolResult, McpError> {
        // 记录 UI/UX 上下文控制信号，便于审计排查
        if request.uiux_intent.is_some()
            || request.uiux_context_policy.is_some()
            || request.uiux_reason.is_some()
        {
            log::info!(
                "UI/UX 上下文信号: intent={:?}, policy={:?}, reason={:?}",
                request.uiux_intent.as_deref(),
                request.uiux_context_policy.as_deref(),
                request.uiux_reason.as_deref()
            );
        }

        log_important!(
            info,
            "[zhi] 记录请求: request_id={}, brief_len={}, brief_preview={}, choices_len={}, workspace={:?}",
            request_id,
            request.brief.len(),
            safe_truncate_clean(&request.brief, 200),
            request.choices.len(),
            request.workspace.as_str()
        );

        // 中文说明：MCP 对外字段采用中性命名，内部仍映射到既有弹窗协议以保持 UI 链路稳定。
        let popup_request = PopupRequest {
            id: request_id.clone(),
            message: request.brief,
            predefined_options: if request.choices.is_empty() {
                None
            } else {
                Some(request.choices)
            },
            is_markdown: request.render_markdown,
            project_root_path: Some(request.workspace),
            // 透传 UI/UX 上下文控制信号
            uiux_intent: request.uiux_intent,
            uiux_context_policy: request.uiux_context_policy,
            uiux_reason: request.uiux_reason,
        };

        match create_tauri_popup(&popup_request) {
            Ok(response) => {
                log_debug!(
                    "[zhi] 弹窗响应已收到: request_id={}, response_len={}",
                    request_id,
                    response.len()
                );
                // 解析响应内容，支持文本和图片
                let content = parse_mcp_response(&response)?;
                Ok(CallToolResult::success(content))
            }
            Err(e) => {
                log_important!(warn, "[zhi] 弹窗失败: request_id={}, error={}", request_id, e);
                Err(popup_error(e.to_string()).into())
            }
        }
    }
}
