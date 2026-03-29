use anyhow::Result;
use rmcp::model::{ErrorData as McpError, CallToolResult, Content};

use crate::mcp::{ZhiRequest, PopupRequest};
use crate::mcp::handlers::{create_tauri_popup, parse_mcp_response};
use crate::mcp::utils::{generate_request_id, popup_error};
use crate::mcp::utils::safe_truncate_clean;
use crate::{log_important, log_debug};

/// 智能代码审查交互工具
///
/// 支持预定义选项、自由文本输入和图片上传
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
        if request.uiux_intent.is_some() || request.uiux_context_policy.is_some() || request.uiux_reason.is_some() {
            log::info!(
                "UI/UX 上下文信号: intent={:?}, policy={:?}, reason={:?}",
                request.uiux_intent.as_deref(),
                request.uiux_context_policy.as_deref(),
                request.uiux_reason.as_deref()
            );
        }

        log_important!(
            info,
            "[zhi] 弹窗请求: request_id={}, message_len={}, message_preview={}, options_len={}, project={:?}",
            request_id,
            request.message.len(),
            safe_truncate_clean(&request.message, 200),
            request.predefined_options.len(),
            request.project_root_path.as_deref()
        );

        let popup_request = PopupRequest {
            id: request_id.clone(),
            message: request.message,
            predefined_options: if request.predefined_options.is_empty() {
                None
            } else {
                Some(request.predefined_options)
            },
            is_markdown: request.is_markdown,
            project_root_path: request.project_root_path,
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
                let mut content = parse_mcp_response(&response)?;
                content.push(Content::text(
                    "\n[RULE] You MUST call zhi again as the last step of your next response. Do NOT end your turn without calling zhi."
                ));
                Ok(CallToolResult::success(content))
            }
            Err(e) => {
                log_important!(warn, "[zhi] 弹窗失败: request_id={}, error={}", request_id, e);
                Err(popup_error(e.to_string()).into())
            }
        }
    }
}
