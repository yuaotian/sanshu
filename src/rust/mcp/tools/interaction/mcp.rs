use anyhow::Result;
use rmcp::model::{
    CallToolResult, ErrorData as McpError, ProgressNotificationParam, ProgressToken,
};
use rmcp::service::Peer;
use rmcp::RoleServer;
use tokio::time::{interval, Duration, MissedTickBehavior};

use crate::mcp::handlers::{create_tauri_popup_async, parse_mcp_response_with_structured};
use crate::mcp::utils::safe_truncate_clean;
use crate::mcp::utils::{generate_request_id, normalize_zhi_choices, popup_error};
use crate::mcp::{PopupRequest, ZhiRequest};
use crate::{log_debug, log_important};

/// Cursor 空闲超时约 120 秒；30 秒心跳留足余量重置计时器
const ZHI_PROGRESS_HEARTBEAT_SECS: u64 = 30;

/// 代码审阅记录工具
///
/// 汇总审阅内容、候选处理项与结构化反馈
#[derive(Clone)]
pub struct InteractionTool;

impl InteractionTool {
    pub async fn zhi(request: ZhiRequest) -> Result<CallToolResult, McpError> {
        // 默认生成 request_id（MCP server 会优先使用其 call_id 注入到 zhi_with_request_id）
        let request_id = generate_request_id();
        Self::zhi_with_request_id(request, request_id, None).await
    }

    /// 带 request_id 的 zhi 调用入口
    ///
    /// 中文说明：用于将 MCP 分发层生成的 call_id 贯穿到 GUI 进程与响应，便于全链路日志关联。
    /// `progress` 存在时，等待弹窗期间周期性发送 MCP progress，避免 Cursor 120 秒空闲超时。
    pub async fn zhi_with_request_id(
        request: ZhiRequest,
        request_id: String,
        progress: Option<(Peer<RoleServer>, ProgressToken)>,
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
            "[zhi] 记录请求: request_id={}, brief_len={}, brief_preview={}, choices_len={}, workspace={}, has_progress_token={}",
            request_id,
            request.brief.len(),
            safe_truncate_clean(&request.brief, 200),
            request.choices.len(),
            request.workspace.as_str(),
            progress.is_some()
        );

        // 中文说明：MCP 对外字段采用中性命名，内部仍映射到既有弹窗协议以保持 UI 链路稳定。
        let choices = normalize_zhi_choices(request.choices);

        let popup_request = PopupRequest {
            id: request_id.clone(),
            message: request.brief,
            predefined_options: if choices.is_empty() {
                None
            } else {
                Some(choices)
            },
            is_markdown: request.render_markdown,
            project_root_path: Some(request.workspace),
            agent_label: request.agent_label,
            // 透传 UI/UX 上下文控制信号
            uiux_intent: request.uiux_intent,
            uiux_context_policy: request.uiux_context_policy,
            uiux_reason: request.uiux_reason,
        };

        let popup_fut = create_tauri_popup_async(&popup_request);
        let response = if let Some((peer, token)) = progress {
            tokio::select! {
                result = popup_fut => result,
                _ = send_zhi_progress_heartbeat(peer, token) => unreachable!("心跳循环不应自行结束"),
            }
        } else {
            popup_fut.await
        };

        match response {
            Ok(response) => {
                log_debug!(
                    "[zhi] 弹窗响应已收到: request_id={}, response_len={}",
                    request_id,
                    response.len()
                );
                // 解析响应内容，支持文本、图片与 structured_content，避免记忆上下文混入 user_input。
                let parsed = parse_mcp_response_with_structured(&response)?;
                Ok(CallToolResult {
                    content: parsed.content,
                    is_error: Some(false),
                    structured_content: parsed.structured_content,
                    meta: None,
                })
            }
            Err(e) => {
                log_important!(
                    warn,
                    "[zhi] 弹窗失败: request_id={}, error={}",
                    request_id,
                    e
                );
                Err(popup_error(e.to_string()).into())
            }
        }
    }
}

/// 等待用户确认期间周期性发送 progress，重置 Cursor 空闲超时
async fn send_zhi_progress_heartbeat(peer: Peer<RoleServer>, token: ProgressToken) {
    let mut ticks: u64 = 0;
    let mut ticker = interval(Duration::from_secs(ZHI_PROGRESS_HEARTBEAT_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        let waited_secs = ticks.saturating_mul(ZHI_PROGRESS_HEARTBEAT_SECS);
        ticks = ticks.saturating_add(1);

        let params = ProgressNotificationParam {
            progress_token: token.clone(),
            progress: ticks as f64,
            total: None,
            message: Some(format!("等待用户确认（已等待 {} 秒）", waited_secs)),
        };

        match peer.notify_progress(params).await {
            Ok(()) => {
                log_important!(
                    info,
                    "[zhi] 已发送 progress 心跳: tick={}, waited_secs={}",
                    ticks,
                    waited_secs
                );
            }
            Err(e) => {
                log_important!(warn, "[zhi] 发送 progress 心跳失败: {}", e);
            }
        }
    }
}
