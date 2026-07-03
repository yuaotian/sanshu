// 图标工坊弹窗处理器
// 负责调用 GUI 进程打开图标选择界面
//
// IPC 协议（与 zhi 弹窗 popup.rs 对齐）：
// 1. 请求侧：TuRequest 序列化写入临时文件，通过 --icon-request <文件> 传给 GUI
// 2. 响应侧：GUI 通过 stdout 返回结构化 IconPopupResponse（含 status 字段），
//    显式区分 saved/cancelled/error，替代旧的"空 stdout = 取消"脆弱约定
// 3. 超时保护：tokio 异步等待 + 可配置超时，避免 GUI 挂起导致 MCP 请求永久阻塞

use anyhow::Result;
use std::fs;
use std::time::{Duration, Instant};

// 复用公共 UI 启动器模块，消除与 popup.rs 的重复代码
use super::ui_launcher::find_ui_command;
use crate::mcp::types::{IconPopupResponse, IconSaveResponse, TuRequest};
use crate::mcp::utils::{generate_request_id, safe_truncate_clean};
use crate::{log_debug, log_important};

/// 图标弹窗默认超时时间（用户挑选图标可能较慢，给足 10 分钟）
const ICON_POPUP_TIMEOUT: Duration = Duration::from_secs(600);

/// 创建图标选择弹窗
///
/// 调用 "等一下" GUI 进程，进入图标搜索模式；
/// 请求通过临时文件传递，结果通过 stdout 的结构化 JSON 返回
pub async fn create_icon_popup(request: &TuRequest) -> Result<IconPopupResponse> {
    let start = Instant::now();
    let request_id = generate_request_id();

    log_important!(
        info,
        "[icon_popup] 启动图标弹窗: request_id={}, query={:?}, style={:?}, save_path={:?}, project_root={:?}",
        request_id,
        request
            .query
            .as_deref()
            .map(|s| safe_truncate_clean(s, 120)),
        request
            .style
            .as_deref()
            .map(|s| safe_truncate_clean(s, 120)),
        request
            .save_path
            .as_deref()
            .map(|s| safe_truncate_clean(s, 120)),
        request
            .project_root
            .as_deref()
            .map(|s| safe_truncate_clean(s, 120))
    );

    // 先确认 UI 命令可用，再写入临时文件，避免命令查找失败时残留请求文件
    let command_path = find_ui_command()?;

    // 将请求写入临时文件（对齐 popup.rs 的 --mcp-request 协议）
    let temp_file = std::env::temp_dir().join(format!("icon_request_{}.json", request_id));
    let request_json = serde_json::json!({
        "id": request_id.as_str(),
        "query": request.query.as_deref(),
        "style": request.style.as_deref(),
        "save_path": request.save_path.as_deref(),
        "project_root": request.project_root.as_deref(),
    });
    fs::write(&temp_file, serde_json::to_string_pretty(&request_json)?)?;

    // 异步启动 GUI 进程并带超时等待，避免 GUI 挂起时 MCP 请求永久卡死
    let output_future = tokio::process::Command::new(&command_path)
        .arg("--icon-request")
        .arg(temp_file.to_string_lossy().to_string())
        .output();

    let output = match tokio::time::timeout(ICON_POPUP_TIMEOUT, output_future).await {
        Ok(result) => {
            // 无论成败先清理临时文件
            let _ = fs::remove_file(&temp_file);
            result?
        }
        Err(_) => {
            let _ = fs::remove_file(&temp_file);
            log_important!(
                error,
                "[icon_popup] GUI进程超时: request_id={}, timeout_secs={}",
                request_id,
                ICON_POPUP_TIMEOUT.as_secs()
            );
            anyhow::bail!(
                "图标选择弹窗等待超时（{} 秒），已放弃本次请求",
                ICON_POPUP_TIMEOUT.as_secs()
            );
        }
    };

    let elapsed_ms = start.elapsed().as_millis();
    let exit_code = output.status.code();
    let stdout_len = output.stdout.len();
    let stderr_len = output.stderr.len();

    if !output.status.success() {
        // 非零退出码 = GUI 崩溃/异常，与用户取消明确区分
        let error = String::from_utf8_lossy(&output.stderr);
        log_important!(
            error,
            "[icon_popup] GUI执行失败: request_id={}, exit_code={:?}, stdout_len={}, stderr_len={}, stderr_preview={}, elapsed_ms={}",
            request_id,
            exit_code,
            stdout_len,
            stderr_len,
            safe_truncate_clean(&error, 200),
            elapsed_ms
        );
        anyhow::bail!("图标选择进程失败: {}", error);
    }

    let response_str = String::from_utf8_lossy(&output.stdout);
    let response_str = response_str.trim();

    log_debug!(
        "[icon_popup] GUI执行成功: request_id={}, exit_code={:?}, stdout_len={}, stderr_len={}, elapsed_ms={}",
        request_id,
        exit_code,
        stdout_len,
        stderr_len,
        elapsed_ms
    );

    parse_icon_popup_response(response_str)
}

/// 解析图标弹窗响应
///
/// 优先解析新的结构化格式（含 status 字段）；
/// 兼容旧格式（cancelled 布尔字段）作为过渡；空输出视为用户直接关闭窗口
fn parse_icon_popup_response(response_str: &str) -> Result<IconPopupResponse> {
    // 空输出：用户直接关闭窗口（GUI 正常退出但未发送响应）
    if response_str.is_empty() {
        return Ok(IconPopupResponse::cancelled());
    }

    // 优先解析新的结构化格式
    if let Ok(response) = serde_json::from_str::<IconPopupResponse>(response_str) {
        if !response.status.is_empty() {
            return Ok(response);
        }
    }

    // 回退：兼容旧格式（前端升级期间的过渡）
    if let Ok(legacy) = serde_json::from_str::<IconSaveResponse>(response_str) {
        return Ok(IconPopupResponse::from_legacy(legacy));
    }

    log_important!(
        error,
        "[icon_popup] 解析响应失败: stdout_preview={}",
        safe_truncate_clean(response_str, 200)
    );
    anyhow::bail!("解析图标保存响应失败：输出不是有效的结构化 JSON")
}
