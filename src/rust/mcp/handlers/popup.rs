use anyhow::Result;
use std::process::Command;
use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::mcp::types::PopupRequest;
use crate::mcp::utils::safe_truncate_clean;
use crate::{log_important, log_debug};

/// 创建 Tauri 弹窗
///
/// 优先调用与 MCP 服务器同目录的 UI 命令，找不到时使用全局版本
pub fn create_tauri_popup(request: &PopupRequest) -> Result<String> {
    let start = Instant::now();
    let request_id = request.id();
    let popup_type = request.popup_type_name();

    // 创建临时请求文件 - 跨平台适配
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("mcp_request_{}.json", request_id));
    let request_json = serde_json::to_string_pretty(request)?;
    fs::write(&temp_file, &request_json)?;

    log_important!(
        info,
        "[popup] 已写入MCP请求文件: request_id={}, popup_type={}, file={}, json_len={}",
        request_id,
        popup_type,
        temp_file.display(),
        request_json.len()
    );

    // 尝试找到等一下命令的路径
    let command_path = find_ui_command()?;

    log_debug!(
        "[popup] 准备调用GUI进程: request_id={}, popup_type={}, command_path={}",
        request_id,
        popup_type,
        command_path
    );

    // 调用等一下命令
    let output = Command::new(&command_path)
        .arg("--mcp-request")
        .arg(temp_file.to_string_lossy().to_string())
        .output()?;

    // 清理临时文件
    let _ = fs::remove_file(&temp_file);

    let elapsed_ms = start.elapsed().as_millis();
    let exit_code = output.status.code();
    let stdout_len = output.stdout.len();
    let stderr_len = output.stderr.len();

    if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);
        let response = response.trim();

        log_important!(
            info,
            "[popup] GUI执行成功: request_id={}, popup_type={}, exit_code={:?}, stdout_len={}, stderr_len={}, elapsed_ms={}",
            request_id,
            popup_type,
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
            "[popup] GUI执行失败: request_id={}, popup_type={}, exit_code={:?}, stdout_len={}, stderr_len={}, stderr_preview={}, elapsed_ms={}",
            request_id,
            popup_type,
            exit_code,
            stdout_len,
            stderr_len,
            safe_truncate_clean(&error, 200),
            elapsed_ms
        );
        anyhow::bail!("UI进程失败: {}", error);
    }
}

/// 查找等一下 UI 命令的路径
///
/// 按优先级查找：同目录 -> 全局版本 -> 开发环境
fn find_ui_command() -> Result<String> {
    // 1. 优先尝试与当前 MCP 服务器同目录的等一下命令
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            let local_ui_path = exe_dir.join("等一下");
            if local_ui_path.exists() && is_executable(&local_ui_path) {
                return Ok(local_ui_path.to_string_lossy().to_string());
            }
        }
    }

    // 2. 尝试全局命令（最常见的部署方式）
    if test_command_available("等一下") {
        return Ok("等一下".to_string());
    }

    // 3. 如果都找不到，返回详细错误信息
    anyhow::bail!(
        "找不到等一下 UI 命令。请确保：\n\
         1. 已编译项目：cargo build --release\n\
         2. 或已全局安装：./install.sh\n\
         3. 或等一下命令在同目录下"
    )
}

/// 测试命令是否可用
fn test_command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// 检查文件是否可执行
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        // Windows 上检查文件扩展名
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("exe"))
            .unwrap_or(false)
    }
}
