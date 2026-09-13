use anyhow::Result;
use std::fs;
use std::process::{Output, Stdio};
use std::time::Instant;

// 复用公共 UI 启动器模块，消除与 icon_popup.rs 的重复代码
use super::ui_launcher::find_ui_command;
use crate::mcp::types::PopupRequest;
use crate::mcp::utils::safe_truncate_clean;
use crate::{log_debug, log_important};

/// 创建 Tauri 弹窗（同步）
///
/// 供 CLI / 设置页测试弹窗使用。MCP `zhi` 请走异步版本，避免阻塞运行时导致无法发送 progress。
pub fn create_tauri_popup(request: &PopupRequest) -> Result<String> {
    let start = Instant::now();
    let (command_path, temp_file) = prepare_popup_request(request)?;

    let output = std::process::Command::new(&command_path)
        .arg("--mcp-request")
        .arg(temp_file.to_string_lossy().to_string())
        .output();

    let _ = fs::remove_file(&temp_file);
    interpret_gui_output(&request.id, output?, start.elapsed().as_millis())
}

/// 创建 Tauri 弹窗（异步）
///
/// 中文说明：用 tokio 等待 GUI，方便 MCP 侧在等待期间发送 progress 心跳。
pub async fn create_tauri_popup_async(request: &PopupRequest) -> Result<String> {
    let start = Instant::now();
    let (command_path, temp_file) = prepare_popup_request(request)?;

    // 中文说明：切断 stdin，避免 GUI 占用 MCP 的 stdio 管道导致心跳发不出去。
    let output = tokio::process::Command::new(&command_path)
        .arg("--mcp-request")
        .arg(temp_file.to_string_lossy().to_string())
        .stdin(Stdio::null())
        .output()
        .await;

    let _ = fs::remove_file(&temp_file);
    interpret_gui_output(&request.id, output?, start.elapsed().as_millis())
}

/// 写入临时请求文件并解析 GUI 命令路径
fn prepare_popup_request(request: &PopupRequest) -> Result<(String, std::path::PathBuf)> {
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("mcp_request_{}.json", request.id));
    let request_json = serde_json::to_string_pretty(request)?;
    fs::write(&temp_file, request_json)?;

    log_important!(
        info,
        "[popup] 已写入MCP请求文件: request_id={}, file={}, message_len={}, message_preview={}, options_len={}, project={:?}, markdown={}",
        request.id,
        temp_file.display(),
        request.message.len(),
        safe_truncate_clean(&request.message, 200),
        request.predefined_options.as_ref().map(|v| v.len()).unwrap_or(0),
        request.project_root_path.as_deref(),
        request.is_markdown
    );

    let command_path = find_ui_command()?;
    log_debug!(
        "[popup] 准备调用GUI进程: request_id={}, command_path={}",
        request.id,
        command_path
    );
    Ok((command_path, temp_file))
}

/// 解析 GUI 进程 stdout/stderr
fn interpret_gui_output(request_id: &str, output: Output, elapsed_ms: u128) -> Result<String> {
    let exit_code = output.status.code();
    let stdout_len = output.stdout.len();
    let stderr_len = output.stderr.len();

    if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);
        let response = response.trim();

        log_important!(
            info,
            "[popup] GUI执行成功: request_id={}, exit_code={:?}, stdout_len={}, stderr_len={}, elapsed_ms={}",
            request_id,
            exit_code,
            stdout_len,
            stderr_len,
            elapsed_ms
        );
        if response.is_empty() {
            Ok("用户取消了操作".to_string())
        } else {
            Ok(response.to_string())
        }
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        log_important!(
            error,
            "[popup] GUI执行失败: request_id={}, exit_code={:?}, stdout_len={}, stderr_len={}, stderr_preview={}, elapsed_ms={}",
            request_id,
            exit_code,
            stdout_len,
            stderr_len,
            safe_truncate_clean(&error, 200),
            elapsed_ms
        );
        anyhow::bail!("UI进程失败: {}", error);
    }
}
