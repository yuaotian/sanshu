use crate::config::load_standalone_telegram_config;
use crate::mcp::types::PopupRequest;
use crate::mcp::utils::generate_request_id;
use crate::telegram::handle_telegram_only_mcp_request;
use crate::log_important;
use crate::app::builder::run_tauri_app;
use anyhow::Result;

/// 处理命令行参数
pub fn handle_cli_args() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    crate::log_debug!("CLI启动参数: {:?}", args);

    match args.len() {
        // 无参数：正常启动GUI
        1 => {
            crate::log_debug!("进入GUI模式（无参数）");
            #[cfg(windows)]
            cleanup_stale_webview2_dirs();
            run_tauri_app();
        }
        // 单参数：帮助或版本
        2 => {
            match args[1].as_str() {
                "--help" | "-h" => print_help(),
                "--version" | "-v" => print_version(),
                _ => {
                    eprintln!("未知参数: {}", args[1]);
                    print_help();
                    std::process::exit(1);
                }
            }
        }
        // 多参数：MCP请求模式或CLI交互模式
        _ => {
            if args[1] == "--mcp-request" {
                if args.len() >= 3 {
                    crate::log_important!(info, "进入MCP请求模式: request_file={}", args[2]);
                    handle_mcp_request(&args[2])?;
                } else {
                    eprintln!("缺少必填参数: --mcp-request <文件>");
                    print_help();
                    std::process::exit(2);
                }
            } else if args[1] == "--cli" {
                // CLI 模式：解析参数并启动 GUI 交互
                crate::log_important!(info, "进入CLI交互模式（--cli）");
                handle_cli_mode(&args[2..])?;
            } else {
                eprintln!("无效的命令行参数");
                print_help();
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// 处理 CLI 交互模式
///
/// 解析参数并设置环境变量，启动 GUI 进入 zhi 交互模式
fn handle_cli_mode(args: &[String]) -> Result<()> {
    #[cfg(windows)]
    setup_isolated_webview2_data_dir();

    // 解析参数
    let mut message: Option<String> = None;
    let mut options: Vec<String> = Vec::new();
    let mut is_markdown = true;
    let mut project_root: Option<String> = None;
    let mut uiux_intent: Option<String> = None;
    let mut uiux_context_policy: Option<String> = None;
    let mut uiux_reason: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--message" | "-m" if i + 1 < args.len() => {
                message = Some(args[i + 1].clone());
                i += 2;
            }
            "--options" | "-o" if i + 1 < args.len() => {
                let raw = args[i + 1].clone();
                options.extend(split_cli_options(&raw));
                i += 2;
            }
            "--option" if i + 1 < args.len() => {
                options.push(args[i + 1].clone());
                i += 2;
            }
            "--markdown" => {
                is_markdown = true;
                i += 1;
            }
            "--no-markdown" => {
                is_markdown = false;
                i += 1;
            }
            "--project-root" if i + 1 < args.len() => {
                project_root = Some(args[i + 1].clone());
                i += 2;
            }
            "--uiux-intent" if i + 1 < args.len() => {
                uiux_intent = Some(args[i + 1].to_lowercase());
                i += 2;
            }
            "--uiux-context-policy" if i + 1 < args.len() => {
                uiux_context_policy = Some(args[i + 1].to_lowercase());
                i += 2;
            }
            "--uiux-reason" if i + 1 < args.len() => {
                uiux_reason = Some(args[i + 1].clone());
                i += 2;
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {
                eprintln!("无效的命令行参数: {}", args[i]);
                print_help();
                std::process::exit(2);
            }
        }
    }

    // 校验必填参数
    let message = match message {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("缺少必填参数: --message");
            print_help();
            std::process::exit(2);
        }
    };

    // 严格校验 UI/UX 参数
    if let Some(ref intent) = uiux_intent {
        if !matches!(
            intent.as_str(),
            "none" | "beautify" | "page_refactor" | "uiux_search"
        ) {
            eprintln!("无效的 --uiux-intent: {}", intent);
            std::process::exit(2);
        }
    }
    if let Some(ref policy) = uiux_context_policy {
        if !matches!(policy.as_str(), "auto" | "force" | "forbid") {
            eprintln!("无效的 --uiux-context-policy: {}", policy);
            std::process::exit(2);
        }
    }

    // 记录 UI/UX 上下文控制信号，便于审计排查
    if uiux_intent.is_some() || uiux_context_policy.is_some() || uiux_reason.is_some() {
        log_important!(
            info,
            "UI/UX 上下文信号: intent={:?}, policy={:?}, reason={:?}",
            uiux_intent.as_deref(),
            uiux_context_policy.as_deref(),
            uiux_reason.as_deref()
        );
    }

    // 构建请求并写入环境变量，供前端读取
    let request = PopupRequest::Zhi {
        id: generate_request_id(),
        message,
        predefined_options: if options.is_empty() { None } else { Some(options) },
        is_markdown,
        project_root_path: project_root,
        uiux_intent,
        uiux_context_policy,
        uiux_reason,
    };
    let request_json = serde_json::to_string(&request)?;
    std::env::set_var("SANSHU_CLI_MODE", "true");
    std::env::set_var("SANSHU_CLI_REQUEST", request_json);

    // 启动 GUI 进入交互模式
    run_tauri_app();
    Ok(())
}

/// 拆分 CLI 选项列表
fn split_cli_options(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// 清理已结束弹窗进程留下的 WebView2 临时目录
#[cfg(windows)]
fn cleanup_stale_webview2_dirs() {
    let temp = std::env::temp_dir();
    if let Ok(entries) = std::fs::read_dir(&temp) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("sanshu_wv2_") {
                if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                    crate::log_debug!("[webview2] 清理临时目录失败（可能仍在使用）: {} - {}", name_str, e);
                }
            }
        }
    }
}

/// 为弹窗进程设置独立的 WebView2 数据目录，避免多实例锁竞争导致黑屏
#[cfg(windows)]
fn setup_isolated_webview2_data_dir() {
    let pid = std::process::id();
    let data_dir = std::env::temp_dir().join(format!("sanshu_wv2_{}", pid));
    let arg = format!("--user-data-dir={}", data_dir.display());
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", &arg);
    log_important!(info, "[webview2] 已设置独立数据目录: {}", data_dir.display());
}

/// 处理MCP请求
fn handle_mcp_request(request_file: &str) -> Result<()> {
    log_important!(info, "[handle_mcp_request] 收到请求文件: {}", request_file);

    #[cfg(windows)]
    setup_isolated_webview2_data_dir();

    // 检查Telegram配置，决定是否启用纯Telegram模式
    match load_standalone_telegram_config() {
        Ok(telegram_config) => {
            if telegram_config.enabled && telegram_config.hide_frontend_popup {
                // 纯Telegram模式：不启动GUI，直接处理
                log_important!(
                    info,
                    "[handle_mcp_request] 进入纯Telegram模式（hide_frontend_popup=true）"
                );
                if let Err(e) = tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(handle_telegram_only_mcp_request(request_file))
                {
                    log_important!(error, "处理Telegram请求失败: {}", e);
                    std::process::exit(1);
                }
            } else {
                // 正常模式：启动GUI处理弹窗
                log_important!(
                    info,
                    "[handle_mcp_request] 进入GUI模式处理弹窗（telegram_enabled={}, hide_frontend_popup={}）",
                    telegram_config.enabled,
                    telegram_config.hide_frontend_popup
                );
                run_tauri_app();
            }
        }
        Err(e) => {
            log_important!(warn, "加载Telegram配置失败: {}，使用默认GUI模式", e);
            // 配置加载失败时，使用默认行为（启动GUI）
            run_tauri_app();
        }
    }
    Ok(())
}

/// 显示帮助信息
fn print_help() {
    println!("三术 - 智能代码审查工具");
    println!();
    println!("用法:");
    println!("  等一下                              启动设置界面");
    println!("  等一下 --mcp-request <文件>          处理 MCP 请求（zhi/icon 统一通道）");
    println!("  等一下 --cli [选项]                  命令行独立调用 zhi 交互");
    println!("  等一下 --help                       显示此帮助信息");
    println!("  等一下 --version                    显示版本信息");
    println!();
    println!("CLI 交互选项:");
    println!("  --message, -m <内容>                 必填，弹窗消息");
    println!("  --options, -o <选项1,选项2>           预定义选项（逗号分隔）");
    println!("  --option <选项>                      预定义选项（可重复）");
    println!("  --markdown / --no-markdown           是否按 Markdown 渲染（默认开启）");
    println!("  --project-root <路径>                项目根目录");
    println!("  --uiux-intent <值>                   none/beautify/page_refactor/uiux_search");
    println!("  --uiux-context-policy <值>           auto/force/forbid");
    println!("  --uiux-reason <内容>                  UI/UX 上下文追加原因");
}

/// 显示版本信息
fn print_version() {
    println!("三术 v{}", env!("CARGO_PKG_VERSION"));
}

