use tauri::{AppHandle, Emitter, State};
use once_cell::sync::Lazy;
use std::env;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config::{AppState, save_config};
use crate::network::proxy::{ProxyDetector, ProxyInfo, ProxyType};
use super::AcemcpTool;
use super::types::{AcemcpRequest, ProjectIndexStatus, ProjectsIndexStatus, ProjectFilesStatus, DetectedProxy, ProxySpeedTestResult, SpeedTestMetric, SpeedTestProgress, SpeedTestStageStatus, ProjectWithNestedStatus};
use reqwest;
use crate::{log_debug, log_important};

#[derive(Debug, Clone, serde::Serialize)]
struct AcemcpLogStreamEvent {
    /// 事件类型: "append" | "error" | "reset"
    event_type: String,
    /// 新增日志行（不含换行符）
    lines: Vec<String>,
    /// 错误信息（仅 error 类型有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AcemcpLogTargetInfo {
    /// 目标标识：combined | current | backup:1 ...
    pub target: String,
    /// 前端展示用标签
    pub label: String,
    /// 是否存在（combined 恒为 true）
    pub exists: bool,
    /// 文件大小（字节，combined 无值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// 修改时间（UTC ISO8601，combined 无值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_utc: Option<String>,
}

static ACEMCP_LOG_STREAM_CANCEL_FLAG: Lazy<Mutex<Option<Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(None));

fn get_acemcp_log_path() -> Result<std::path::PathBuf, String> {
    if let Ok(path) = env::var("MCP_LOG_FILE") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(std::path::PathBuf::from(trimmed));
        }
    }

    // 使用 dirs::config_dir() 获取系统配置目录，确保跨平台兼容性
    // Windows: C:\Users\<用户>\AppData\Roaming\sanshu\log\acemcp.log
    // Linux: ~/.config/sanshu/log/acemcp.log
    // macOS: ~/Library/Application Support/sanshu/log/acemcp.log
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "无法获取系统配置目录，请检查操作系统环境".to_string())?;

    Ok(config_dir.join("sanshu").join("log").join("acemcp.log"))
}

fn ensure_acemcp_log_dir_exists(log_path: &Path) -> Result<(), String> {
    // 确保日志目录存在
    if let Some(log_dir) = log_path.parent() {
        if !log_dir.exists() {
            std::fs::create_dir_all(log_dir)
                .map_err(|e| format!("创建日志目录失败: {} (路径: {})", e, log_dir.display()))?;
        }
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct SaveAcemcpConfigArgs {
    #[serde(alias = "baseUrl", alias = "base_url")]
    pub base_url: String,
    #[serde(alias = "token", alias = "_token")]
    pub token: String,
    #[serde(alias = "batchSize", alias = "batch_size")]
    pub batch_size: u32,
    #[serde(alias = "maxLinesPerBlob", alias = "_max_lines_per_blob")]
    pub max_lines_per_blob: u32,
    #[serde(alias = "textExtensions", alias = "_text_extensions")]
    pub text_extensions: Vec<String>,
    #[serde(alias = "excludePatterns", alias = "_exclude_patterns")]
    pub exclude_patterns: Vec<String>,
    #[serde(alias = "watchDebounceMs", alias = "watch_debounce_ms")]
    pub watch_debounce_ms: Option<u64>, // 文件监听防抖延迟（毫秒）
    // 代理配置
    #[serde(alias = "proxyEnabled", alias = "proxy_enabled")]
    pub proxy_enabled: Option<bool>,
    #[serde(alias = "proxyHost", alias = "proxy_host")]
    pub proxy_host: Option<String>,
    #[serde(alias = "proxyPort", alias = "proxy_port")]
    pub proxy_port: Option<u16>,
    #[serde(alias = "proxyType", alias = "proxy_type")]
    pub proxy_type: Option<String>,
    #[serde(alias = "proxyUsername", alias = "proxy_username")]
    pub proxy_username: Option<String>,
    #[serde(alias = "proxyPassword", alias = "proxy_password")]
    pub proxy_password: Option<String>,
    /// 是否自动索引嵌套的 Git 子项目
    #[serde(alias = "indexNestedProjects", alias = "index_nested_projects")]
    pub index_nested_projects: Option<bool>,
    /// 是否在对话窗口顶部显示索引状态面板（默认关闭）
    #[serde(alias = "showIndexPanel", alias = "show_index_panel")]
    pub show_index_panel: Option<bool>,
}


#[tauri::command]
pub async fn save_acemcp_config(
    args: SaveAcemcpConfigArgs,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    // 规范化 base_url：补充协议（如缺失）并去除末尾斜杠，防止URL拼接时出现双斜杠
    let mut base_url = args.base_url.trim().to_string();
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        base_url = format!("http://{}", base_url);
        log::warn!("BASE_URL 缺少协议，已自动补全为: {}", base_url);
    }
    // 去除末尾的所有斜杠，确保URL格式统一
    while base_url.ends_with('/') {
        base_url.pop();
    }
    log::info!("规范化后的 BASE_URL: {}", base_url);

    {
        let mut config = state
            .config
            .lock()
            .map_err(|e| format!("获取配置失败: {}", e))?;

        config.mcp_config.acemcp_base_url = Some(base_url.clone());
        config.mcp_config.acemcp_token = Some(args.token.clone());
        config.mcp_config.acemcp_batch_size = Some(args.batch_size);
        config.mcp_config.acemcp_max_lines_per_blob = Some(args.max_lines_per_blob);
        config.mcp_config.acemcp_text_extensions = Some(args.text_extensions.clone());
        config.mcp_config.acemcp_exclude_patterns = Some(args.exclude_patterns.clone());
        config.mcp_config.acemcp_watch_debounce_ms = args.watch_debounce_ms;
        // 保存代理配置
        config.mcp_config.acemcp_proxy_enabled = args.proxy_enabled;
        config.mcp_config.acemcp_proxy_host = args.proxy_host.clone();
        config.mcp_config.acemcp_proxy_port = args.proxy_port;
        config.mcp_config.acemcp_proxy_type = args.proxy_type.clone();
        config.mcp_config.acemcp_proxy_username = args.proxy_username.clone();
        config.mcp_config.acemcp_proxy_password = args.proxy_password.clone();
        // 保存嵌套项目索引开关
        // 仅在前端显式传入时才覆盖，避免其他页面保存配置时将用户设置重置为默认值
        if let Some(v) = args.index_nested_projects {
            config.mcp_config.acemcp_index_nested_projects = Some(v);
        }
        if let Some(v) = args.show_index_panel {
            config.mcp_config.acemcp_show_index_panel = Some(v);
        }
    }

    save_config(&state, &app)
        .await
        .map_err(|e| format!("保存配置失败: {}", e))?;

    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct TestAcemcpArgs {
    #[serde(alias = "baseUrl", alias = "base_url")]
    pub base_url: String,
    #[serde(alias = "token", alias = "_token")]
    pub token: String,
}

#[derive(Debug, serde::Serialize)]
pub struct TestConnectionResult {
    pub success: bool,
    pub message: String,
}

#[tauri::command]
pub async fn test_acemcp_connection(
    args: TestAcemcpArgs,
    state: State<'_, AppState>,
) -> Result<TestConnectionResult, String> {
    // 获取配置并立即释放锁
    let (
        effective_base_url,
        effective_token,
        proxy_enabled,
        proxy_host,
        proxy_port,
        proxy_type,
        proxy_username,
        proxy_password,
    ) = {
        let config = state.config
            .lock()
            .map_err(|e| format!("获取配置失败: {}", e))?;
        
        let base_url = config.mcp_config.acemcp_base_url.as_ref().unwrap_or(&args.base_url).clone();
        let token = config.mcp_config.acemcp_token.as_ref().unwrap_or(&args.token).clone();

        // 代理配置（连接测试也需要遵循“所有 ACE 通信走代理”的要求）
        let proxy_enabled = config.mcp_config.acemcp_proxy_enabled.unwrap_or(false);
        let proxy_host = config.mcp_config.acemcp_proxy_host.clone().unwrap_or_else(|| "127.0.0.1".to_string());
        let proxy_port = config.mcp_config.acemcp_proxy_port.unwrap_or(7890);
        let proxy_type = config.mcp_config.acemcp_proxy_type.clone().unwrap_or_else(|| "http".to_string());
        let proxy_username = config.mcp_config.acemcp_proxy_username.clone();
        let proxy_password = config.mcp_config.acemcp_proxy_password.clone();

        (
            base_url,
            token,
            proxy_enabled,
            proxy_host,
            proxy_port,
            proxy_type,
            proxy_username,
            proxy_password,
        )
    };
    
    // 验证 URL 格式
    if !effective_base_url.starts_with("http://") && !effective_base_url.starts_with("https://") {
        let msg = "无效的API端点URL格式，必须以 http:// 或 https:// 开头".to_string();
        return Ok(TestConnectionResult {
            success: false,
            message: msg,
        });
    }
    
    // 验证 token
    if effective_token.trim().is_empty() {
        let msg = "认证令牌不能为空".to_string();
        return Ok(TestConnectionResult {
            success: false,
            message: msg,
        });
    }
    
    // 规范化 base_url
    let normalized_url = if effective_base_url.ends_with('/') {
        effective_base_url[..effective_base_url.len() - 1].to_string()
    } else {
        effective_base_url.clone()
    };
    
    // 实际测试连接 - 发送一个简单的健康检查请求
    let mut client_builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10));

    // 如果启用代理，则连接测试也走代理（避免“设置了代理但测试仍失败”的误导）
    if proxy_enabled {
        // 校验代理类型，避免拼接出无效 URL
        match proxy_type.as_str() {
            "http" | "https" | "socks5" => {}
            other => {
                return Ok(TestConnectionResult {
                    success: false,
                    message: format!("不支持的代理类型: {}（仅支持 http/https/socks5）", other),
                });
            }
        }

        let proxy_url = format!("{}://{}:{}", proxy_type, proxy_host, proxy_port);
        let mut reqwest_proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| format!("创建代理失败: {}", e))?;

        // 代理认证（Basic Auth）
        if let Some(username) = proxy_username.as_deref() {
            let username = username.trim();
            if !username.is_empty() {
                let password = proxy_password.as_deref().unwrap_or("");
                reqwest_proxy = reqwest_proxy.basic_auth(username, password);
            }
        }

        client_builder = client_builder.proxy(reqwest_proxy);
    }

    let client = client_builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    
    // 尝试访问一个常见的端点（如果存在健康检查端点）
    let test_url = format!("{}/health", normalized_url);
    
    match client
        .get(&test_url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", effective_token))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            
            if status.is_success() {
                let msg = format!("连接测试成功！API 端点响应正常 (HTTP {})", status.as_u16());
                return Ok(TestConnectionResult {
                    success: true,
                    message: msg,
                });
            }
        }
        Err(_) => {
            // 健康检查端点可能不存在，继续测试实际 API 端点
        }
    }
    
    // 如果健康检查失败，尝试测试实际的代码库检索端点
    let search_url = format!("{}/agents/codebase-retrieval", normalized_url);
    
    // 发送一个最小的测试请求
    let test_payload = serde_json::json!({
        "information_request": "test",
        "blobs": {"checkpoint_id": null, "added_blobs": [], "deleted_blobs": []},
        "dialog": [],
        "max_output_length": 0,
        "disable_codebase_retrieval": false,
        "enable_commit_retrieval": false,
    });
    
    match client
        .post(&search_url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", effective_token))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&test_payload)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            
            if status.is_success() {
                let msg = format!("连接测试成功！API 端点响应正常 (HTTP {})", status.as_u16());
                Ok(TestConnectionResult {
                    success: true,
                    message: msg,
                })
            } else {
                let body = response.text().await.unwrap_or_default();
                let msg = format!("API 端点返回错误状态: {} {}", status.as_u16(), status.as_str());
                Ok(TestConnectionResult {
                    success: false,
                    message: format!("{} - 响应: {}", msg, if body.len() > 200 { format!("{}...", &body[..200]) } else { body }),
                })
            }
        }
        Err(e) => {
            let msg = format!("连接失败: {}", e);
            Ok(TestConnectionResult {
                success: false,
                message: msg,
            })
        }
    }
}

#[tauri::command]
pub async fn get_acemcp_log_directory() -> Result<String, String> {
    let log_path = get_acemcp_log_path()?;
    ensure_acemcp_log_dir_exists(&log_path)?;

    let dir = log_path
        .parent()
        .ok_or_else(|| format!("日志路径无效: {}", log_path.display()))?;
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_acemcp_log_file_path() -> Result<String, String> {
    let log_path = get_acemcp_log_path()?;
    ensure_acemcp_log_dir_exists(&log_path)?;
    Ok(log_path.to_string_lossy().to_string())
}

/// 获取可用的日志目标列表（用于前端下拉切换）
#[tauri::command]
pub async fn list_acemcp_log_targets() -> Result<Vec<AcemcpLogTargetInfo>, String> {
    let log_path = get_acemcp_log_path()?;
    ensure_acemcp_log_dir_exists(&log_path)?;

    let max_backups: usize = crate::utils::logger::LogRotationConfig::default().max_backup_count as usize;

    let log_dir = log_path
        .parent()
        .ok_or_else(|| format!("日志路径无效: {}", log_path.display()))?;
    let log_name = log_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("acemcp.log");

    let mut out: Vec<AcemcpLogTargetInfo> = Vec::new();

    // 合并视图
    out.push(AcemcpLogTargetInfo {
        target: "combined".to_string(),
        label: "合并视图（备份 + 当前）".to_string(),
        exists: true,
        size_bytes: None,
        modified_utc: None,
    });

    // 当前文件
    let current_exists = log_path.exists();
    let (size_bytes, modified_utc) = if current_exists {
        let meta = std::fs::metadata(&log_path).ok();
        let size = meta.as_ref().map(|m| m.len());
        let modified = meta
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        (size, modified)
    } else {
        (None, None)
    };
    out.push(AcemcpLogTargetInfo {
        target: "current".to_string(),
        label: log_name.to_string(),
        exists: current_exists,
        size_bytes,
        modified_utc,
    });

    // 轮转备份：.1 .. .N（仅返回存在的文件）
    for i in 1..=max_backups {
        let path = log_dir.join(format!("{}.{}", log_name, i));
        if !path.exists() {
            continue;
        }
        let meta = std::fs::metadata(&path).ok();
        let size_bytes = meta.as_ref().map(|m| m.len());
        let modified_utc = meta
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

        out.push(AcemcpLogTargetInfo {
            target: format!("backup:{}", i),
            label: format!("{}.{}", log_name, i),
            exists: true,
            size_bytes,
            modified_utc,
        });
    }

    Ok(out)
}

/// 读取日志文件内容
#[tauri::command]
pub async fn read_acemcp_logs(
    _state: State<'_, AppState>,
    max_lines: Option<usize>,
    target: Option<String>,
) -> Result<Vec<String>, String> {
    let log_path = get_acemcp_log_path()?;
    ensure_acemcp_log_dir_exists(&log_path)?;

    // 读取当前日志 + 轮转备份（acemcp.log.1, acemcp.log.2 ...），返回最后 N 行（默认 1000，最大 5000）
    // 中文说明：使用流式读取避免在日志很大时一次性读入内存。
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader};

    // 前端日志查看器默认 5000 行；为了避免误传导致卡顿，这里做上限保护。
    let max_lines: usize = max_lines.unwrap_or(1000).clamp(100, 5000);
    let max_backups: usize = crate::utils::logger::LogRotationConfig::default().max_backup_count as usize;
    let target = target.unwrap_or_else(|| "combined".to_string());

    let log_dir = log_path
        .parent()
        .ok_or_else(|| format!("日志路径无效: {}", log_path.display()))?;
    let log_name = log_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("acemcp.log");

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    match target.as_str() {
        "" | "combined" => {
            // 从最旧备份到最新：.N ... .1
            for i in (1..=max_backups).rev() {
                candidates.push(log_dir.join(format!("{}.{}", log_name, i)));
            }
            // 最后读取当前文件
            candidates.push(log_path.clone());
        }
        "current" => {
            candidates.push(log_path.clone());
        }
        t if t.starts_with("backup:") => {
            let idx = t.strip_prefix("backup:").unwrap_or("");
            let i: usize = idx.parse::<usize>().unwrap_or(0);
            if i == 0 || i > max_backups {
                // 无效备份编号：按空处理
                return Ok(vec![]);
            }
            candidates.push(log_dir.join(format!("{}.{}", log_name, i)));
        }
        other => {
            return Err(format!("无效的日志目标: {}", other));
        }
    }

    let mut buf: VecDeque<String> = VecDeque::with_capacity(max_lines);
    let mut any_exists = false;

    for path in candidates {
        if !path.exists() {
            continue;
        }
        any_exists = true;

        // 轮转/写入过程中可能出现短暂的占用或读取失败，这里采用 best-effort：跳过无法读取的文件。
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let bytes = match reader.read_until(b'\n', &mut line) {
                Ok(n) => n,
                Err(_) => break,
            };
            if bytes == 0 {
                break;
            }

            while matches!(line.last(), Some(b'\n') | Some(b'\r')) {
                line.pop();
            }

            let text = String::from_utf8_lossy(&line).to_string();
            if buf.len() == max_lines {
                buf.pop_front();
            }
            buf.push_back(text);
        }
    }

    if !any_exists {
        return Ok(vec![]);
    }

    Ok(buf.into_iter().collect())
}

/// 导出日志到文件（写入日志目录，返回导出文件路径）
///
/// 中文说明：
/// - 默认导出“前端传入的 lines”（通常是过滤后的可见日志），最多建议 5000 行
/// - 支持 txt/csv（csv 会写入 UTF-8 BOM 以兼容 Windows Excel）
#[tauri::command]
pub async fn export_acemcp_logs(
    format: String,
    lines: Vec<String>,
    file_name_hint: Option<String>,
) -> Result<String, String> {
    use std::io::Write;

    let log_path = get_acemcp_log_path()?;
    ensure_acemcp_log_dir_exists(&log_path)?;

    let log_dir = log_path
        .parent()
        .ok_or_else(|| format!("日志路径无效: {}", log_path.display()))?;

    let format = format.trim().to_lowercase();
    let ext = match format.as_str() {
        "txt" => "txt",
        "csv" => "csv",
        other => return Err(format!("不支持的导出格式: {}（仅支持 txt/csv）", other)),
    };

    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();

    // 中文说明：文件名只做最小化清洗，避免路径注入
    let safe_hint = file_name_hint
        .unwrap_or_else(|| "acemcp-export".to_string())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();

    let export_path = log_dir.join(format!("{}-{}.{}", safe_hint, ts, ext));
    let line_count = lines.len();

    // best-effort：即使没有 lines 也允许导出空文件
    let file = std::fs::File::create(&export_path)
        .map_err(|e| format!("创建导出文件失败: {} (路径: {})", e, export_path.display()))?;
    let mut writer = std::io::BufWriter::new(file);

    if ext == "txt" {
        for line in lines {
            writer
                .write_all(line.as_bytes())
                .map_err(|e| format!("写入导出文件失败: {}", e))?;
            writer
                .write_all(b"\n")
                .map_err(|e| format!("写入导出文件失败: {}", e))?;
        }
        writer.flush().ok();
        log_important!(
            info,
            "[log_export] 导出完成: format=txt, lines={}, path={}",
            line_count,
            export_path.display()
        );
        return Ok(export_path.to_string_lossy().to_string());
    }

    // CSV 导出（带 UTF-8 BOM，兼容 Excel）
    writer
        .write_all(b"\xEF\xBB\xBF")
        .map_err(|e| format!("写入 BOM 失败: {}", e))?;

    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(writer);

    let re = regex::Regex::new(
        r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[([A-Z]+)\] \[([^\]]+)\] (.*)$",
    )
    .map_err(|e| format!("构建日志解析正则失败: {}", e))?;

    // 表头
    wtr.write_record(["timestamp", "level", "module", "message", "raw"])
        .map_err(|e| format!("写入 CSV 表头失败: {}", e))?;

    for raw in lines {
        if let Some(caps) = re.captures(&raw) {
            let ts = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let level = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let module = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            let msg = caps.get(4).map(|m| m.as_str()).unwrap_or("");
            wtr.write_record([ts, level, module, msg, raw.as_str()])
                .map_err(|e| format!("写入 CSV 失败: {}", e))?;
        } else {
            wtr.write_record(["", "", "", raw.as_str(), raw.as_str()])
                .map_err(|e| format!("写入 CSV 失败: {}", e))?;
        }
    }

    wtr.flush().map_err(|e| format!("写入 CSV 失败: {}", e))?;

    // 写入路径日志（避免泄露内容）
    log_important!(
        info,
        "[log_export] 导出完成: format=csv, path={}",
        export_path.display()
    );

    Ok(export_path.to_string_lossy().to_string())
}

/// 启动 acemcp 日志流（tail -f）
///
/// 中文说明：
/// - 采用轮询方式读取 acemcp.log 新增内容，跨平台更稳定（Windows 轮转 rename 更容易兼容）
/// - 事件名称固定为 "acemcp-log-stream"，前端按需 listen/unlisten
#[tauri::command]
pub async fn start_acemcp_log_stream(
    app: AppHandle,
    interval_ms: Option<u64>,
) -> Result<(), String> {
    let interval_ms = interval_ms.unwrap_or(300).clamp(50, 2_000);
    let log_path = get_acemcp_log_path()?;
    ensure_acemcp_log_dir_exists(&log_path)?;

    // 如已有旧任务，先取消（避免重复推送）
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut guard = ACEMCP_LOG_STREAM_CANCEL_FLAG
            .lock()
            .map_err(|_| "日志流取消锁已被毒化（poisoned）".to_string())?;
        if let Some(old) = guard.take() {
            old.store(true, Ordering::Relaxed);
        }
        *guard = Some(cancel_flag.clone());
    }

    log_important!(
        info,
        "[log_stream] 启动 acemcp 日志流: interval_ms={}, path={}",
        interval_ms,
        log_path.display()
    );

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use std::io::{BufRead, BufReader, Seek, SeekFrom};

        // 中文说明：首次启动默认从文件末尾开始，只推送“新增日志”
        let mut offset: Option<u64> = None;

        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }

            let meta = match std::fs::metadata(&log_path) {
                Ok(m) => m,
                Err(e) => {
                    // 文件还未生成/临时不可读：best-effort，不刷屏
                    log_debug!("[log_stream] 读取日志文件元信息失败（将重试）: {}", e);
                    tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                    continue;
                }
            };

            let file_len = meta.len();
            match offset {
                None => {
                    offset = Some(file_len);
                }
                Some(off) if file_len < off => {
                    // 轮转或截断：从头开始读新文件
                    log_debug!(
                        "[log_stream] 检测到日志文件轮转/截断，重置 offset: old_offset={}, new_size={}",
                        off,
                        file_len
                    );
                    offset = Some(0);
                    let _ = app.emit(
                        "acemcp-log-stream",
                        &AcemcpLogStreamEvent {
                            event_type: "reset".to_string(),
                            lines: vec![],
                            error: None,
                        },
                    );
                }
                _ => {}
            }

            let mut file = match std::fs::File::open(&log_path) {
                Ok(f) => f,
                Err(e) => {
                    log_debug!("[log_stream] 打开日志文件失败（将重试）: {}", e);
                    tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                    continue;
                }
            };

            let off = offset.unwrap_or(0);
            if file.seek(SeekFrom::Start(off)).is_err() {
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;
                continue;
            }

            let mut reader = BufReader::new(file);
            let mut new_lines: Vec<String> = Vec::new();
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        // 去除末尾换行符
                        while line.ends_with('\n') || line.ends_with('\r') {
                            line.pop();
                        }
                        if !line.is_empty() {
                            new_lines.push(line.clone());
                        }
                    }
                    Err(e) => {
                        let _ = app.emit(
                            "acemcp-log-stream",
                            &AcemcpLogStreamEvent {
                                event_type: "error".to_string(),
                                lines: vec![],
                                error: Some(format!("读取日志失败: {}", e)),
                            },
                        );
                        break;
                    }
                }
            }

            // 更新 offset
            if let Ok(pos) = reader.stream_position() {
                offset = Some(pos);
            }

            if !new_lines.is_empty() {
                let _ = app.emit(
                    "acemcp-log-stream",
                    &AcemcpLogStreamEvent {
                        event_type: "append".to_string(),
                        lines: new_lines,
                        error: None,
                    },
                );
            }

            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }

        log_debug!("[log_stream] acemcp 日志流已停止");
    });

    Ok(())
}

/// 停止 acemcp 日志流（释放后台任务）
#[tauri::command]
pub async fn stop_acemcp_log_stream() -> Result<(), String> {
    let flag = {
        let mut guard = ACEMCP_LOG_STREAM_CANCEL_FLAG
            .lock()
            .map_err(|_| "日志流取消锁已被毒化（poisoned）".to_string())?;
        guard.take()
    };

    if let Some(f) = flag {
        f.store(true, Ordering::Relaxed);
        log_debug!("[log_stream] 收到停止请求，已设置取消标记");
    }

    Ok(())
}

#[tauri::command]
pub async fn clear_acemcp_cache(_state: State<'_, AppState>) -> Result<String, String> {
    // 使用 dirs::home_dir() 获取用户主目录，确保跨平台兼容性
    // 如果获取失败，降级到当前目录（与项目中 home_projects_file() 保持一致）
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let cache_dir = home.join(".acemcp").join("data");

    // 如果缓存目录存在，先删除
    if cache_dir.exists() {
        std::fs::remove_dir_all(&cache_dir)
            .map_err(|e| format!("删除缓存目录失败: {} (路径: {})", e, cache_dir.display()))?;
    }

    // 重新创建缓存目录
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("创建缓存目录失败: {} (路径: {})", e, cache_dir.display()))?;

    let cache_path = cache_dir.to_string_lossy().to_string();
    log::info!("acemcp缓存已清除: {}", cache_path);
    Ok(cache_path)
}

#[derive(Debug, serde::Serialize)]
pub struct AcemcpConfigResponse {
    pub base_url: Option<String>,
    pub token: Option<String>,
    pub batch_size: u32,
    pub max_lines_per_blob: u32,
    pub text_extensions: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub watch_debounce_ms: u64, // 文件监听防抖延迟（毫秒），默认 180000 (3分钟)
    // 代理配置
    pub proxy_enabled: bool,
    pub proxy_host: String,
    pub proxy_port: u16,
    pub proxy_type: String,
    pub proxy_username: String,
    pub proxy_password: String,
    /// 是否自动索引嵌套的 Git 子项目（默认启用）
    pub index_nested_projects: bool,
    /// 是否在对话窗口顶部显示索引状态面板（默认关闭）
    pub show_index_panel: bool,
}

#[tauri::command]
pub async fn get_acemcp_config(state: State<'_, AppState>) -> Result<AcemcpConfigResponse, String> {
    let config = state.config
        .lock()
        .map_err(|e| format!("获取配置失败: {}", e))?;
    Ok(AcemcpConfigResponse {
        base_url: config.mcp_config.acemcp_base_url.clone(),
        token: config.mcp_config.acemcp_token.clone(),
        batch_size: config.mcp_config.acemcp_batch_size.unwrap_or(10),
        max_lines_per_blob: config.mcp_config.acemcp_max_lines_per_blob.unwrap_or(800),
        // 默认文件扩展名列表（与前端 McpToolsTab.vue 保持一致）
        // 用户首次打开设置界面时，所有扩展名默认全部勾选
        text_extensions: config.mcp_config.acemcp_text_extensions.clone().unwrap_or_else(|| {
            vec![
                ".py".to_string(), ".js".to_string(), ".ts".to_string(),
                ".jsx".to_string(), ".tsx".to_string(), ".java".to_string(),
                ".go".to_string(), ".rs".to_string(), ".cpp".to_string(),
                ".c".to_string(), ".h".to_string(), ".hpp".to_string(),
                ".cs".to_string(), ".rb".to_string(), ".php".to_string(),
                ".md".to_string(), ".txt".to_string(), ".json".to_string(),
                ".yaml".to_string(), ".yml".to_string(), ".toml".to_string(),
                ".xml".to_string(), ".html".to_string(), ".css".to_string(),
                ".scss".to_string(), ".sql".to_string(), ".sh".to_string(),
                ".bash".to_string()
            ]
        }),
        exclude_patterns: config.mcp_config.acemcp_exclude_patterns.clone().unwrap_or_else(|| {
            vec!["node_modules".to_string(), ".git".to_string(), "target".to_string(), "dist".to_string()]
        }),
        watch_debounce_ms: config.mcp_config.acemcp_watch_debounce_ms.unwrap_or(180_000),
        // 代理配置
        proxy_enabled: config.mcp_config.acemcp_proxy_enabled.unwrap_or(false),
        proxy_host: config.mcp_config.acemcp_proxy_host.clone().unwrap_or_else(|| "127.0.0.1".to_string()),
        proxy_port: config.mcp_config.acemcp_proxy_port.unwrap_or(7890),
        proxy_type: config.mcp_config.acemcp_proxy_type.clone().unwrap_or_else(|| "http".to_string()),
        proxy_username: config.mcp_config.acemcp_proxy_username.clone().unwrap_or_default(),
        proxy_password: config.mcp_config.acemcp_proxy_password.clone().unwrap_or_default(),
        // 嵌套项目索引开关（默认启用）
        index_nested_projects: config.mcp_config.acemcp_index_nested_projects.unwrap_or(true),
        show_index_panel: config.mcp_config.acemcp_show_index_panel.unwrap_or(false),
    })
}

#[derive(Debug, serde::Serialize)]
pub struct DebugSearchResult {
    /// 搜索是否成功
    pub success: bool,
    /// 搜索结果文本
    pub result: Option<String>,
    /// 错误信息
    pub error: Option<String>,
    /// 请求发送时间 ISO8601 格式
    pub request_time: String,
    /// 响应接收时间 ISO8601 格式
    pub response_time: String,
    /// 总耗时（毫秒）
    pub total_duration_ms: u64,
    /// 搜索结果数量
    pub result_count: Option<usize>,
    /// 项目路径
    pub project_path: String,
    /// 查询语句
    pub query: String,
}

/// 纯 Rust 的调试命令：直接执行 acemcp 搜索，返回结果及耗时统计
#[tauri::command]
pub async fn debug_acemcp_search(
    project_root_path: String,
    query: String,
    _app: AppHandle,
) -> Result<DebugSearchResult, String> {
    use std::time::Instant;
    
    // 记录请求开始时间
    let request_time = chrono::Utc::now();
    let request_time_str = request_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let start_instant = Instant::now();
    
    let req = AcemcpRequest { 
        project_root_path: project_root_path.clone(), 
        query: query.clone() 
    };
    
    // 调用搜索函数（日志会通过 log crate 输出到日志文件）
    log::info!("[调试搜索] 开始执行: project={}, query={}", project_root_path, query);
    let search_result = AcemcpTool::search_context(req).await;
    
    // 记录响应接收时间
    let response_time = chrono::Utc::now();
    let response_time_str = response_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let total_duration_ms = start_instant.elapsed().as_millis() as u64;
    
    log::info!("[调试搜索] 执行完成: 耗时 {}ms", total_duration_ms);
    
    match search_result {
        Ok(result) => {
            let mut result_text = String::new();
            let mut result_count: Option<usize> = None;
            
            if let Ok(val) = serde_json::to_value(&result) {
                if let Some(arr) = val.get("content").and_then(|v| v.as_array()) {
                    result_count = Some(arr.len());
                    for item in arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(txt) = item.get("text").and_then(|t| t.as_str()) {
                                result_text.push_str(txt);
                            }
                        }
                    }
                }
            }
            
            Ok(DebugSearchResult {
                success: true,
                result: Some(result_text),
                error: None,
                request_time: request_time_str,
                response_time: response_time_str,
                total_duration_ms,
                result_count,
                project_path: project_root_path,
                query,
            })
        }
        Err(e) => {
            let error_msg = format!("执行失败: {}", e);
            log::error!("[调试搜索] 错误: {}", error_msg);
            
            Ok(DebugSearchResult {
                success: false,
                result: None,
                error: Some(error_msg),
                request_time: request_time_str,
                response_time: response_time_str,
                total_duration_ms,
                result_count: None,
                project_path: project_root_path,
                query,
            })
        }
    }
}


/// 执行acemcp工具
#[tauri::command]
pub async fn execute_acemcp_tool(
    tool_name: String,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match tool_name.as_str() {
        "search_context" => {
            // 解析参数
            let project_root_path = arguments.get("project_root_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "缺少project_root_path参数".to_string())?
                .to_string();
            
            let query = arguments.get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "缺少query参数".to_string())?
                .to_string();
            
            // 执行搜索
            let req = AcemcpRequest { project_root_path, query };
            match AcemcpTool::search_context(req).await {
                Ok(result) => {
                    // 转换结果为JSON
                    if let Ok(val) = serde_json::to_value(&result) {
                        Ok(serde_json::json!({
                            "status": "success",
                            "result": val
                        }))
                    } else {
                        Err("结果序列化失败".to_string())
                    }
                }
                Err(e) => Ok(serde_json::json!({
                    "status": "error",
                    "error": e.to_string()
                })),
            }
        }
        _ => Err(format!("未知的工具: {}", tool_name)),
    }
}

/// 获取指定项目的索引状态
#[tauri::command]
pub fn get_acemcp_index_status(project_root_path: String) -> Result<ProjectIndexStatus, String> {
    Ok(AcemcpTool::get_index_status(project_root_path))
}

/// 获取所有项目的索引状态
#[tauri::command]
pub fn get_all_acemcp_index_status() -> Result<ProjectsIndexStatus, String> {
    log::debug!("📋 [get_all_acemcp_index_status] 开始获取所有项目索引状态");
    
    let status = AcemcpTool::get_all_index_status();
    let project_count = status.projects.len();
    
    log::debug!("📊 [get_all_acemcp_index_status] 返回项目数: {}", project_count);
    
    // 详细记录每个项目的状态（用于调试）
    for (path, proj_status) in &status.projects {
        log::debug!(
            "📁 [get_all_acemcp_index_status] 项目: path={}, status={:?}, total_files={}, last_success_time={:?}",
            path,
            proj_status.status,
            proj_status.total_files,
            proj_status.last_success_time
        );
    }
    
    Ok(status)
}

/// 获取指定项目内所有可索引文件的索引状态，用于前端构建文件树
#[tauri::command]
pub async fn get_acemcp_project_files_status(
    project_root_path: String,
) -> Result<ProjectFilesStatus, String> {
    AcemcpTool::get_project_files_status(project_root_path)
        .await
        .map_err(|e| e.to_string())
}

/// 获取项目及其嵌套子项目的索引状态
/// 用于前端展示包含多个 Git 子仓库的项目结构
#[tauri::command]
pub fn get_acemcp_project_with_nested(project_root_path: String) -> Result<ProjectWithNestedStatus, String> {
    // 关键校验：目录不存在时直接返回错误
    if !check_directory_exists(project_root_path.clone())? {
        return Err(format!("项目根目录不存在: {}", project_root_path));
    }
    AcemcpTool::get_project_with_nested_status(project_root_path)
        .map_err(|e| e.to_string())
}

/// 手动触发索引更新
#[tauri::command]
pub async fn trigger_acemcp_index_update(project_root_path: String) -> Result<String, String> {
    AcemcpTool::trigger_index_update(project_root_path)
        .await
        .map_err(|e| e.to_string())
}

/// 手动触发索引全量重建
/// 先清理本地索引记录，再重新触发索引
#[tauri::command]
pub async fn trigger_acemcp_index_rebuild(project_root_path: String) -> Result<String, String> {
    // 先清理本地索引记录（projects.json + projects_status.json）
    // 全量重建不主动停止文件监听，避免影响自动索引
    purge_project_index_records(&project_root_path, false)
        .map_err(|e| format!("全量重建前清理索引记录失败: {}", e))?;

    // 再触发索引更新（全量重建）
    AcemcpTool::trigger_index_update(project_root_path)
        .await
        .map_err(|e| e.to_string())
}

/// 获取全局自动索引开关状态
#[tauri::command]
pub fn get_auto_index_enabled() -> Result<bool, String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    Ok(watcher_manager.is_auto_index_enabled())
}

/// 设置全局自动索引开关
#[tauri::command]
pub async fn set_auto_index_enabled(
    enabled: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    watcher_manager.set_auto_index_enabled(enabled);

    // 持久化到配置，确保跨重启生效
    {
        let mut config = state
            .config
            .lock()
            .map_err(|e| format!("获取配置失败: {}", e))?;
        config.mcp_config.acemcp_auto_index_enabled = Some(enabled);
    }

    save_config(&state, &app)
        .await
        .map_err(|e| format!("保存配置失败: {}", e))?;
    Ok(())
}

/// 获取当前正在监听的项目列表
#[tauri::command]
pub fn get_watching_projects() -> Result<Vec<String>, String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    Ok(watcher_manager.get_watching_projects())
}

/// 检查指定项目是否正在监听
#[tauri::command]
pub fn is_project_watching(project_root_path: String) -> Result<bool, String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    Ok(watcher_manager.is_watching(&project_root_path))
}

/// 启动项目文件监听
/// 从配置中读取防抖延迟参数
#[tauri::command]
pub async fn start_project_watching(
    project_root_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 从配置中读取防抖延迟
    let debounce_ms = {
        let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        config.mcp_config.acemcp_watch_debounce_ms
    };
    
    // 获取 acemcp 配置
    let acemcp_config = super::AcemcpTool::get_acemcp_config()
        .await
        .map_err(|e| format!("获取 acemcp 配置失败: {}", e))?;
    
    log::info!("启动项目监听: path={}, debounce_ms={:?}", project_root_path, debounce_ms);
    
    // 启动监听
    let watcher_manager = super::watcher::get_watcher_manager();
    watcher_manager.start_watching(project_root_path, acemcp_config, debounce_ms)
        .await
        .map_err(|e| format!("启动监听失败: {}", e))
}

/// 停止监听指定项目
#[tauri::command]
pub fn stop_project_watching(project_root_path: String) -> Result<(), String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    watcher_manager.stop_watching(&project_root_path)
        .map_err(|e| e.to_string())
}

/// 停止所有项目监听
#[tauri::command]
pub fn stop_all_watching() -> Result<(), String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    watcher_manager.stop_all();
    Ok(())
}

// 辅助函数：规范化路径 key（去除扩展路径前缀，统一使用正斜杠）
fn normalize_path_key(path: &str) -> String {
    let mut normalized = path.to_string();
    // 尝试规范化路径（失败则保持原样）
    if let Ok(canon) = std::path::PathBuf::from(path).canonicalize() {
        normalized = canon.to_string_lossy().to_string();
    }
    // 去除 Windows 扩展长度路径前缀
    if normalized.starts_with("\\\\?\\") || normalized.starts_with("//?/") {
        normalized = normalized[4..].to_string();
    }
    // 统一使用正斜杠
    normalized.replace('\\', "/")
}

/// 清理指定项目的索引记录
/// stop_watching = true 时会停止文件监听
fn purge_project_index_records(project_root_path: &str, stop_watching: bool) -> Result<String, String> {
    use std::path::PathBuf;
    use std::fs;
    use std::collections::HashMap;

    // 规范化传入的路径
    let normalized_root = normalize_path_key(project_root_path);

    log::info!("[purge_project_index_records] 开始清理项目索引记录");
    log::info!("[purge_project_index_records] 原始路径: {}", project_root_path);
    log::info!("[purge_project_index_records] 规范化后路径: {}", normalized_root);

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = home.join(".acemcp").join("data");

    let mut projects_deleted = false;
    let mut status_deleted = false;

    // 1. 从 projects.json 中删除项目的 blob 列表
    let projects_path = data_dir.join("projects.json");
    if projects_path.exists() {
        match fs::read_to_string(&projects_path) {
            Ok(data) => {
                let mut needs_write = false;
                let mut projects: HashMap<String, Vec<String>> = match serde_json::from_str(&data) {
                    Ok(val) => val,
                    Err(e) => {
                        log::warn!("[purge_project_index_records] projects.json 解析失败，将重置文件: {}", e);
                        needs_write = true;
                        projects_deleted = true;
                        HashMap::new()
                    }
                };

                let existing_keys: Vec<&String> = projects.keys().collect();
                log::info!("[purge_project_index_records] projects.json 中现有项目: {:?}", existing_keys);

                let key_to_remove: Option<String> = projects.keys()
                    .find(|k| normalize_path_key(k) == normalized_root)
                    .cloned();

                if let Some(key) = key_to_remove {
                    log::info!("[purge_project_index_records] 找到匹配的 key: {}", key);
                    projects.remove(&key);
                    needs_write = true;
                    projects_deleted = true;
                } else if needs_write {
                    log::warn!("[purge_project_index_records] 未找到匹配项目，但 projects.json 已被重置");
                } else {
                    log::warn!("[purge_project_index_records] ✗ 在 projects.json 中未找到匹配的项目，规范化路径: {}", normalized_root);
                }

                if needs_write {
                    let new_data = serde_json::to_string_pretty(&projects)
                        .map_err(|e| format!("序列化 projects.json 失败: {} (路径: {})", e, projects_path.display()))?;
                    fs::write(&projects_path, new_data)
                        .map_err(|e| format!("写入 projects.json 失败: {} (路径: {})", e, projects_path.display()))?;
                }
            }
            Err(e) => {
                return Err(format!("读取 projects.json 失败: {} (路径: {})", e, projects_path.display()));
            }
        }
    } else {
        log::warn!("[purge_project_index_records] projects.json 文件不存在: {:?}", projects_path);
    }

    // 2. 从 projects_status.json 中删除项目状态
    let status_path = data_dir.join("projects_status.json");
    if status_path.exists() {
        match fs::read_to_string(&status_path) {
            Ok(data) => {
                let mut needs_write = false;
                let mut status: ProjectsIndexStatus = match serde_json::from_str(&data) {
                    Ok(val) => val,
                    Err(e) => {
                        log::warn!("[purge_project_index_records] projects_status.json 解析失败，将重置文件: {}", e);
                        needs_write = true;
                        status_deleted = true;
                        ProjectsIndexStatus::default()
                    }
                };

                let existing_keys: Vec<&String> = status.projects.keys().collect();
                log::info!("[purge_project_index_records] projects_status.json 中现有项目: {:?}", existing_keys);

                if status.projects.remove(&normalized_root).is_some() {
                    needs_write = true;
                    status_deleted = true;
                    log::info!("[purge_project_index_records] ✓ 已从 projects_status.json 删除项目: {}", normalized_root);
                } else if needs_write {
                    log::warn!("[purge_project_index_records] 未找到匹配项目，但 projects_status.json 已被重置");
                } else {
                    log::warn!("[purge_project_index_records] ✗ 在 projects_status.json 中未找到匹配的项目，规范化路径: {}", normalized_root);
                }

                if needs_write {
                    let new_data = serde_json::to_string_pretty(&status)
                        .map_err(|e| format!("序列化 projects_status.json 失败: {} (路径: {})", e, status_path.display()))?;
                    fs::write(&status_path, new_data)
                        .map_err(|e| format!("写入 projects_status.json 失败: {} (路径: {})", e, status_path.display()))?;
                }
            }
            Err(e) => {
                return Err(format!("读取 projects_status.json 失败: {} (路径: {})", e, status_path.display()));
            }
        }
    } else {
        log::warn!("[purge_project_index_records] projects_status.json 文件不存在: {:?}", status_path);
    }

    // 3. 视需要停止该项目的文件监听
    if stop_watching {
        let watcher_manager = super::watcher::get_watcher_manager();
        let _ = watcher_manager.stop_watching(&normalized_root);
    }

    if projects_deleted || status_deleted {
        log::info!("[purge_project_index_records] 清理完成: projects.json={}, status.json={}", projects_deleted, status_deleted);
        Ok(format!("已清理项目索引记录: {}", normalized_root))
    } else {
        log::warn!("[purge_project_index_records] 未能从任何文件中删除项目，可能路径不匹配");
        Ok(format!("项目索引记录可能已不存在: {}", normalized_root))
    }
}

/// 删除指定项目的索引记录
/// 同时清理 projects.json 和 projects_status.json 中的数据
#[tauri::command]
pub async fn remove_acemcp_project_index(project_root_path: String) -> Result<String, String> {
    purge_project_index_records(&project_root_path, true)
}

/// 检查指定目录是否存在
#[tauri::command]
pub fn check_directory_exists(directory_path: String) -> Result<bool, String> {
    use std::path::PathBuf;

    let path = PathBuf::from(&directory_path);
    
    // 尝试规范化路径（处理 Windows 扩展路径前缀等情况）
    let normalized = path.canonicalize().unwrap_or(path.clone());
    
    Ok(normalized.exists() && normalized.is_dir())
}

// ============ 代理检测和测速命令 ============

/// 自动检测本地可用的代理
/// 返回所有检测到的可用代理列表
#[tauri::command]
pub async fn detect_acemcp_proxy(extra_ports: Option<Vec<u16>>) -> Result<Vec<DetectedProxy>, String> {
    log::info!("🔍 开始检测本地代理...");
    
    // 常用代理端口列表
    let mut ports_to_check: Vec<(u16, &'static str)> = vec![
        (7890, "http"),   // Clash 混合端口
        (7891, "http"),   // Clash HTTP 端口
        (10808, "http"),  // V2Ray HTTP 端口
        (10809, "socks5"), // V2Ray SOCKS5 端口
        (1080, "socks5"), // 通用 SOCKS5 端口
        (8080, "http"),   // 通用 HTTP 代理端口
    ];
    
    // 追加用户自定义端口（同时尝试 http 与 socks5）
    if let Some(extra) = extra_ports {
        let mut seen: std::collections::HashSet<(u16, &'static str)> =
            ports_to_check.iter().copied().collect();

        for port in extra {
            if port == 0 {
                continue;
            }

            for proxy_type_str in ["http", "socks5"] {
                if seen.insert((port, proxy_type_str)) {
                    ports_to_check.push((port, proxy_type_str));
                }
            }
        }
    }

    // 并发检测所有端口（符合需求：并发检测 + 3 秒超时由 ProxyDetector 内部保证）
    let mut tasks = tokio::task::JoinSet::new();
    for (port, proxy_type_str) in ports_to_check {
        tasks.spawn(async move {
            let proxy_type = if proxy_type_str == "socks5" {
                ProxyType::Socks5
            } else {
                ProxyType::Http
            };

            let proxy_info = ProxyInfo::new(proxy_type, "127.0.0.1".to_string(), port);
            let start = std::time::Instant::now();

            if ProxyDetector::check_proxy(&proxy_info).await {
                let response_time = start.elapsed().as_millis() as u64;
                log::info!(
                    "✅ 检测到可用代理: 127.0.0.1:{} ({}), 响应时间: {}ms",
                    port,
                    proxy_type_str,
                    response_time
                );

                Some(DetectedProxy {
                    host: "127.0.0.1".to_string(),
                    port,
                    proxy_type: proxy_type_str.to_string(),
                    response_time_ms: Some(response_time),
                })
            } else {
                None
            }
        });
    }

    let mut detected_proxies: Vec<DetectedProxy> = Vec::new();
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Some(p)) => detected_proxies.push(p),
            Ok(None) => {}
            Err(e) => log::debug!("代理检测任务异常（忽略，不影响整体结果）: {}", e),
        }
    }
    
    // 按响应时间排序
    detected_proxies.sort_by(|a, b| {
        a.response_time_ms.unwrap_or(u64::MAX).cmp(&b.response_time_ms.unwrap_or(u64::MAX))
    });
    
    log::info!("🔍 代理检测完成，找到 {} 个可用代理", detected_proxies.len());
    Ok(detected_proxies)
}

/// 代理测速命令
/// 测试代理和直连模式下的网络延迟和搜索性能
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn test_acemcp_proxy_speed(
    app: AppHandle,               // 用于发送进度事件
    test_mode: String,            // "proxy" | "direct" | "compare"
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_type: Option<String>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
    test_query: String,
    project_root_path: String,
    project_upload_mode: Option<String>,      // "sample" | "full"
    project_upload_max_files: Option<u32>,    // 采样模式下的文件上限
    state: State<'_, AppState>,
) -> Result<ProxySpeedTestResult, String> {
    log::info!("🚀 [SpeedTest] 开始代理测速");
    log::info!("📋 [SpeedTest] 参数: mode={}, query={}, project={}", test_mode, test_query, project_root_path);
    
    // 进度发送辅助闭包
    let emit_progress = |stage: u8, stage_name: &str, percentage: u8, status: SpeedTestStageStatus, detail: Option<&str>, sub_step: Option<&str>| {
        let progress = SpeedTestProgress {
            stage,
            stage_name: stage_name.to_string(),
            percentage,
            status,
            detail: detail.map(|s| s.to_string()),
            sub_step: sub_step.map(|s| s.to_string()),
        };
        let _ = app.emit("speed_test_progress", &progress);
    };
    
    // 阶段0: 开始初始化
    emit_progress(0, "初始化", 0, SpeedTestStageStatus::Running, Some("正在加载配置..."), None);
    
    // 获取配置
    let (base_url, token, batch_size, max_lines_per_blob) = {
        let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        (
            config.mcp_config.acemcp_base_url.clone().ok_or("未配置租户地址")?,
            config.mcp_config.acemcp_token.clone().ok_or("未配置 ACE Token")?,
            config.mcp_config.acemcp_batch_size.unwrap_or(10) as usize,
            config.mcp_config.acemcp_max_lines_per_blob.unwrap_or(800) as usize,
        )
    };
    
    log::debug!("⚙️ [SpeedTest] 配置: base_url={}, batch_size={}, max_lines={}", base_url, batch_size, max_lines_per_blob);
    
    let mut metrics: Vec<SpeedTestMetric> = Vec::new();
    let test_proxy = test_mode == "proxy" || test_mode == "compare";
    let test_direct = test_mode == "direct" || test_mode == "compare";
    
    log::info!("🔧 [SpeedTest] 测试模式: test_proxy={}, test_direct={}", test_proxy, test_direct);
    
    // 构建代理信息
    let proxy_info = if test_proxy {
        let host = proxy_host.clone().unwrap_or_else(|| "127.0.0.1".to_string());
        let port = proxy_port.unwrap_or(7890);
        let p_type = proxy_type.clone().unwrap_or_else(|| "http".to_string());
        Some(DetectedProxy {
            host: host.clone(),
            port,
            proxy_type: p_type.clone(),
            response_time_ms: None,
        })
    } else {
        None
    };
    
    if let Some(ref pi) = proxy_info {
        log::info!("🔌 [SpeedTest] 代理配置: {}://{}:{}", pi.proxy_type, pi.host, pi.port);
    }

    // 构建代理设置（用于实际 HTTP 请求，支持 https + 认证）
    let proxy_settings = proxy_info.as_ref().map(|pi| ProxySettings {
        proxy_type: pi.proxy_type.clone(),
        host: pi.host.clone(),
        port: pi.port,
        username: proxy_username.clone(),
        password: proxy_password.clone(),
    });

    // 项目上传测速策略（按 zhi 确认：默认采样，可全量/可自定义上限）
    let project_upload_mode = project_upload_mode
        .unwrap_or_else(|| "sample".to_string())
        .to_lowercase();
    let sample_max_files = project_upload_max_files.unwrap_or(200).max(1) as usize;
    let project_upload_max_files_limit: Option<usize> = match project_upload_mode.as_str() {
        "full" => None,
        "sample" => Some(sample_max_files),
        other => return Err(format!("无效的项目上传模式: {}（仅支持 sample/full）", other)),
    };

    // 读取测试项目文件列表（用于上传测速）
    let project_root_path = project_root_path.trim().to_string();
    let mut project_files_status: Option<ProjectFilesStatus> = None;
    let mut project_files_error: Option<String> = None;

    if project_root_path.is_empty() {
        project_files_error = Some("未选择测试项目，已跳过上传测试".to_string());
        log::warn!("⚠️ [SpeedTest] 未选择测试项目，跳过上传测试");
    } else {
        log::debug!("📂 [SpeedTest] 获取项目文件列表: {}", project_root_path);
        match AcemcpTool::get_project_files_status(project_root_path.clone()).await {
            Ok(v) => {
                if v.files.is_empty() {
                    project_files_error = Some("测试项目未发现可索引文件，已跳过上传测试".to_string());
                    log::warn!("⚠️ [SpeedTest] 测试项目无可索引文件");
                } else {
                    log::info!("📁 [SpeedTest] 项目文件数: {} 个", v.files.len());
                    project_files_status = Some(v);
                }
            }
            Err(e) => {
                project_files_error = Some(format!("获取测试项目文件列表失败: {}", e));
                log::error!("❌ [SpeedTest] 获取项目文件列表失败: {}", e);
            }
        }
    }

    // 构建测速 HTTP Client（复用连接池 + connect_timeout）
    // 说明：测速过程中会多次请求，如果每次都 build client 会有额外开销
    let proxy_client: Option<reqwest::Client> = if test_proxy {
        if let Some(ref ps) = proxy_settings {
            Some(build_speed_test_client(Some(ps), 120)?)
        } else {
            None
        }
    } else {
        None
    };

    let direct_client: Option<reqwest::Client> = if test_direct {
        Some(build_speed_test_client(None, 120)?)
    } else {
        None
    };
    
    log::info!("🔗 [SpeedTest] HTTP Client 初始化完成: proxy_client={}, direct_client={}", 
               proxy_client.is_some(), direct_client.is_some());
    
    // 阶段0: 初始化完成
    let file_count = project_files_status.as_ref().map(|p| p.files.len()).unwrap_or(0);
    emit_progress(0, "初始化", 10, SpeedTestStageStatus::Completed, 
                  Some(&format!("项目文件: {} 个, HTTP Client 就绪", file_count)), None);
    
    // 1. Ping 测试 - 测量到 ACE 服务器的网络延迟
    let health_url = format!("{}/health", base_url);
    let mut ping_metric = SpeedTestMetric {
        name: "🌐 网络延迟".to_string(),
        metric_type: "ping".to_string(),
        proxy_time_ms: None,
        direct_time_ms: None,
        success: true,
        error: None,
        search_result_preview: None,
    };
    
    log::info!("📡 [SpeedTest] === 阶段1: Ping 测试 ===");
    log::debug!("📡 [SpeedTest] Ping URL: {}", health_url);
    
    // 阶段1: Ping 测试开始
    emit_progress(1, "Ping 测试", 12, SpeedTestStageStatus::Running, 
                  Some("正在测试网络连通性..."), None);
    
    // 代理模式 Ping
    if test_proxy {
        emit_progress(1, "Ping 测试", 15, SpeedTestStageStatus::Running, 
                      Some("代理模式 Ping..."), Some("代理 Ping"));
        if let Some(ref client) = proxy_client {
            let rounds = 3usize;
            let mut ok: Vec<u64> = Vec::with_capacity(rounds);
            let mut last_err: Option<String> = None;

            for _ in 0..rounds {
                match ping_endpoint(client, &health_url, &token).await {
                    Ok(ms) => ok.push(ms),
                    Err(e) => last_err = Some(e),
                }
            }

            if ok.is_empty() {
                ping_metric.success = false;
                append_error(&mut ping_metric.error, format!("代理 Ping 失败: {}", last_err.unwrap_or_else(|| "未知错误".to_string())));
            } else {
                let avg = ok.iter().sum::<u64>() / ok.len() as u64;
                ping_metric.proxy_time_ms = Some(avg);
                if ok.len() != rounds {
                    ping_metric.success = false;
                    append_error(
                        &mut ping_metric.error,
                        format!(
                            "代理 Ping 部分失败: 成功 {}/{}，最后错误: {}",
                            ok.len(),
                            rounds,
                            last_err.unwrap_or_else(|| "未知错误".to_string())
                        ),
                    );
                }
            }
            log::info!("📡 [SpeedTest] 代理 Ping 完成: avg={}ms, success={}/{}", 
                       ping_metric.proxy_time_ms.unwrap_or(0), ok.len(), rounds);
            
            emit_progress(1, "Ping 测试", 20, SpeedTestStageStatus::Running, 
                          Some(&format!("代理 Ping: avg={}ms, {}/{}", ping_metric.proxy_time_ms.unwrap_or(0), ok.len(), rounds)), 
                          Some("代理 Ping 完成"));
        } else {
            ping_metric.success = false;
            append_error(&mut ping_metric.error, "代理 Ping 跳过：代理 client 未初始化".to_string());
        }
    }
    
    // 直连模式 Ping
    if test_direct {
        emit_progress(1, "Ping 测试", 25, SpeedTestStageStatus::Running, 
                      Some("直连模式 Ping..."), Some("直连 Ping"));
        let direct_client = direct_client.as_ref().ok_or_else(|| "直连 Ping 跳过：直连 client 未初始化".to_string())?;
        let rounds = 3usize;
        let mut ok: Vec<u64> = Vec::with_capacity(rounds);
        let mut last_err: Option<String> = None;

        for _ in 0..rounds {
            match ping_endpoint(direct_client, &health_url, &token).await {
                Ok(ms) => ok.push(ms),
                Err(e) => last_err = Some(e),
            }
        }

        if ok.is_empty() {
            ping_metric.success = false;
            append_error(&mut ping_metric.error, format!("直连 Ping 失败: {}", last_err.unwrap_or_else(|| "未知错误".to_string())));
        } else {
            let avg = ok.iter().sum::<u64>() / ok.len() as u64;
            ping_metric.direct_time_ms = Some(avg);
            if ok.len() != rounds {
                ping_metric.success = false;
                append_error(
                    &mut ping_metric.error,
                    format!(
                        "直连 Ping 部分失败: 成功 {}/{}，最后错误: {}",
                        ok.len(),
                        rounds,
                        last_err.unwrap_or_else(|| "未知错误".to_string())
                    ),
                );
            }
        }
        log::info!("📡 [SpeedTest] 直连 Ping 完成: avg={}ms, success={}/{}", 
                   ping_metric.direct_time_ms.unwrap_or(0), ok.len(), rounds);
        
        emit_progress(1, "Ping 测试", 30, SpeedTestStageStatus::Running, 
                      Some(&format!("直连 Ping: avg={}ms, {}/{}", ping_metric.direct_time_ms.unwrap_or(0), ok.len(), rounds)), 
                      Some("直连 Ping 完成"));
    }
    metrics.push(ping_metric);
    
    // Ping 阶段完成
    emit_progress(1, "Ping 测试", 35, SpeedTestStageStatus::Completed, 
                  Some(&format!("代理: {}ms, 直连: {}ms", 
                               metrics.last().and_then(|m| m.proxy_time_ms).unwrap_or(0),
                               metrics.last().and_then(|m| m.direct_time_ms).unwrap_or(0))), 
                  None);
    
    // 2. 语义搜索测试（支持多条查询：按换行/分号分隔）
    let search_url = format!("{}/agents/codebase-retrieval", base_url);
    
    // 从 projects.json 加载测试项目的 blob_names（与 mcp.rs::search_only 保持一致）
    let mut blob_names: Vec<String> = {
        use std::path::PathBuf;
        
        let projects_path = super::mcp::home_projects_file();
        let projects: super::mcp::ProjectsFile = if projects_path.exists() {
            let data = std::fs::read_to_string(&projects_path).unwrap_or_default();
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            super::mcp::ProjectsFile::default()
        };
        
        let normalized_root = PathBuf::from(&project_root_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&project_root_path))
            .to_string_lossy()
            .replace('\\', "/");
        
        projects.0.get(&normalized_root).cloned().unwrap_or_default()
    };

    let current_scope_hash = super::mcp::build_index_scope_hash(&super::AcemcpTool::get_acemcp_config().await
        .map_err(|e| format!("获取 acemcp 配置失败: {}", e))?);
    let project_status = super::AcemcpTool::get_index_status(project_root_path.clone());
    let scope_changed = match current_scope_hash.as_deref() {
        Some(current_hash) => match project_status.index_scope_hash.as_deref() {
            Some(saved_hash) => saved_hash != current_hash,
            None => !blob_names.is_empty(),
        },
        None => false,
    };
    if scope_changed {
        // 中文注释：测速也必须遵循当前索引空间，避免对已失效的旧 blob 做误导性测试。
        log::warn!("⚠️ [SpeedTest] 检测到 ACE 配置变更，已忽略旧索引 blob 列表");
        blob_names.clear();
    }
    
    log::info!("🔍 [SpeedTest] 加载项目 blob_names: 数量={}", blob_names.len());
    
    if blob_names.is_empty() {
        log::warn!("⚠️ [SpeedTest] 项目未索引或索引为空，搜索测试可能返回空结果");
    }

    let mut queries: Vec<String> = test_query
        .split('\n')
        .flat_map(|line| line.split(';'))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    if queries.is_empty() {
        queries.push("代码搜索测试".to_string());
    }

    // 防止用户输入过多查询导致请求数量过大
    const MAX_QUERIES: usize = 5;
    if queries.len() > MAX_QUERIES {
        queries.truncate(MAX_QUERIES);
        log::warn!("⚠️ [SpeedTest] 查询数量超限，已截断为 {} 条", MAX_QUERIES);
    }
    
    log::info!("🔍 [SpeedTest] === 阶段2: 语义搜索测试 ===");
    log::info!("🔍 [SpeedTest] 搜索 URL: {}", search_url);
    log::info!("🔍 [SpeedTest] 查询数量: {} 条", queries.len());
    
    // 阶段2: 语义搜索开始
    emit_progress(2, "语义搜索", 40, SpeedTestStageStatus::Running, 
                  Some(&format!("查询数: {} 条, 索引: {} blobs", queries.len(), blob_names.len())), 
                  None);

    for q in queries {
        let display_q = if q.len() > 30 {
            format!("{}...", &q[..30])
        } else {
            q.clone()
        };
        
        log::debug!("🔎 [SpeedTest] 执行搜索: {}", display_q);

        let mut search_metric = SpeedTestMetric {
            name: format!("🔍 语义搜索 {}", display_q),
            metric_type: "search".to_string(),
            proxy_time_ms: None,
            direct_time_ms: None,
            success: true,
            error: None,
            search_result_preview: None,
        };

        let search_payload = serde_json::json!({
            "information_request": q,
            "blobs": {
                "checkpoint_id": serde_json::Value::Null, 
                "added_blobs": blob_names.clone(),  // 使用已索引的 blob_names
                "deleted_blobs": []
            },
            "dialog": [],
            "max_output_length": 0,  // 与 search_only 保持一致
            "disable_codebase_retrieval": false,
            "enable_commit_retrieval": false,
        });

        // 代理模式搜索
        if test_proxy {
            if let Some(ref client) = proxy_client {
                match search_endpoint(client, &search_url, &token, &search_payload).await {
                    Ok(result) => {
                        search_metric.proxy_time_ms = Some(result.elapsed_ms);
                        // 优先使用代理模式的搜索结果预览
                        if search_metric.search_result_preview.is_none() {
                            search_metric.search_result_preview = result.preview;
                        }
                    }
                    Err(e) => {
                        search_metric.success = false;
                        search_metric.error = Some(format!("代理搜索失败: {}", e));
                        log::error!("❌ [SpeedTest] 代理搜索失败: {}", e);
                    }
                }
            } else {
                search_metric.success = false;
                search_metric.error = Some("代理搜索跳过：代理 client 未初始化".to_string());
            }
        }

        // 直连模式搜索
        if test_direct {
            let direct_client = direct_client.as_ref().ok_or_else(|| "直连搜索跳过：直连 client 未初始化".to_string())?;
            match search_endpoint(direct_client, &search_url, &token, &search_payload).await {
                Ok(result) => {
                    search_metric.direct_time_ms = Some(result.elapsed_ms);
                    // 如果代理模式没有预览，使用直连模式的
                    if search_metric.search_result_preview.is_none() {
                        search_metric.search_result_preview = result.preview;
                    }
                }
                Err(e) => {
                    if search_metric.error.is_none() {
                        search_metric.success = false;
                        search_metric.error = Some(format!("直连搜索失败: {}", e));
                    }
                }
            }
        }
        
        log::info!("🔍 [SpeedTest] 搜索完成 '{}': proxy={}ms, direct={}ms", 
                   display_q, 
                   search_metric.proxy_time_ms.map_or("-".to_string(), |v| v.to_string()),
                   search_metric.direct_time_ms.map_or("-".to_string(), |v| v.to_string()));
        
        // 输出搜索结果预览摘要
        if let Some(ref preview) = search_metric.search_result_preview {
            log::info!("📄 [SpeedTest] 搜索结果: 匹配数={}, 响应长度={}B", 
                       preview.total_matches, preview.response_length);
            // 输出第一个片段的预览（截断显示）
            if let Some(first_snippet) = preview.snippets.first() {
                let snippet_preview = if first_snippet.snippet.len() > 100 {
                    format!("{}...", &first_snippet.snippet[..100])
                } else {
                    first_snippet.snippet.clone()
                };
                // 去除换行符以便日志更整洁
                let snippet_oneline = snippet_preview.replace('\n', " ↵ ");
                log::debug!("📝 [SpeedTest] 首个片段: file={}, content={}", 
                           first_snippet.file_path, snippet_oneline);
            }
        } else {
            log::debug!("📝 [SpeedTest] 未获取到搜索结果预览");
        }

        metrics.push(search_metric);
    }
    
    // 阶段2: 语义搜索完成
    emit_progress(2, "语义搜索", 55, SpeedTestStageStatus::Completed, 
                  Some("所有搜索查询完成"), None);

    // 3. 单文件上传测试（真实走 /batch-upload）
    let mut upload_single_metric = SpeedTestMetric {
        name: "📤 单文件上传".to_string(),
        metric_type: "upload_single".to_string(),
        proxy_time_ms: None,
        direct_time_ms: None,
        success: true,
        error: None,
        search_result_preview: None,
    };
    
    log::info!("📤 [SpeedTest] === 阶段3: 单文件上传测试 ===");
    
    // 阶段3: 单文件上传开始
    emit_progress(3, "单文件上传", 60, SpeedTestStageStatus::Running, 
                  Some("正在上传测试文件..."), None);

    if let Some(err) = project_files_error.clone() {
        upload_single_metric.success = false;
        upload_single_metric.error = Some(err);
    } else if let Some(ref pfs) = project_files_status {
        // 随机选择一个文件进行单文件上传测速（更贴近真实场景）
        if !pfs.files.is_empty() {
            let random_index = fastrand::usize(0..pfs.files.len());
            let file = &pfs.files[random_index];
            match build_single_file_blobs_for_speed_test(&project_root_path, &file.path, max_lines_per_blob) {
                Ok((blobs, file_bytes)) => {
                    let upload_url = format!("{}/batch-upload", base_url);
                    upload_single_metric.name = format!(
                        "📤 单文件上传 ({}，{} blobs)",
                        format_bytes(file_bytes),
                        blobs.len()
                    );
                    
                    log::debug!("📤 [SpeedTest] 单文件: path={}, size={}, blobs={}", 
                               file.path, format_bytes(file_bytes), blobs.len());

                    if test_proxy {
                        if let Some(ref client) = proxy_client {
                            match upload_blobs_batch(client, &upload_url, &token, &blobs, 120).await {
                                Ok(ms) => upload_single_metric.proxy_time_ms = Some(ms),
                                Err(e) => {
                                    upload_single_metric.success = false;
                                    append_error(&mut upload_single_metric.error, format!("代理上传失败: {}", e));
                                }
                            }
                        } else {
                            upload_single_metric.success = false;
                            append_error(&mut upload_single_metric.error, "代理上传跳过：代理 client 未初始化".to_string());
                        }
                    }

                    if test_direct {
                        let direct_client = direct_client.as_ref().ok_or_else(|| "直连上传跳过：直连 client 未初始化".to_string())?;
                        match upload_blobs_batch(direct_client, &upload_url, &token, &blobs, 120).await {
                            Ok(ms) => upload_single_metric.direct_time_ms = Some(ms),
                            Err(e) => {
                                upload_single_metric.success = false;
                                append_error(&mut upload_single_metric.error, format!("直连上传失败: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    upload_single_metric.success = false;
                    upload_single_metric.error = Some(e);
                }
            }
        } else {
            upload_single_metric.success = false;
            upload_single_metric.error = Some("测试项目没有可用文件，已跳过单文件上传测试".to_string());
        }
    }
    metrics.push(upload_single_metric);
    
    // 阶段3: 单文件上传完成
    emit_progress(3, "单文件上传", 70, SpeedTestStageStatus::Completed, 
                  Some(&format!("代理: {}ms, 直连: {}ms", 
                               metrics.last().and_then(|m| m.proxy_time_ms).unwrap_or(0),
                               metrics.last().and_then(|m| m.direct_time_ms).unwrap_or(0))), 
                  None);

    // 4. 项目上传测试（按策略：采样/全量）
    let mut upload_project_metric = SpeedTestMetric {
        name: "📦 项目上传".to_string(),
        metric_type: "upload_project".to_string(),
        proxy_time_ms: None,
        direct_time_ms: None,
        success: true,
        error: None,
        search_result_preview: None,
    };
    
    log::info!("📦 [SpeedTest] === 阶段4: 项目上传测试 ===");
    log::info!("📦 [SpeedTest] 上传模式: {}, 文件上限: {:?}", 
               project_upload_mode, project_upload_max_files_limit);
    
    // 阶段4: 项目上传开始
    emit_progress(4, "项目上传", 75, SpeedTestStageStatus::Running, 
                  Some(&format!("模式: {}, 文件上限: {:?}", project_upload_mode, project_upload_max_files_limit)), 
                  None);

    if let Some(err) = project_files_error.clone() {
        upload_project_metric.success = false;
        upload_project_metric.error = Some(err);
    } else if let Some(ref pfs) = project_files_status {
        let mut detail: Option<ProjectUploadResult> = None;

        if test_proxy {
            if let Some(ref client) = proxy_client {
                match upload_project_for_speed_test(
                    client,
                    &base_url,
                    &token,
                    &project_root_path,
                    pfs,
                    batch_size,
                    max_lines_per_blob,
                    project_upload_max_files_limit,
                )
                .await
                {
                    Ok(r) => {
                        upload_project_metric.proxy_time_ms = Some(r.elapsed_ms);
                        if detail.is_none() {
                            detail = Some(r);
                        }
                    }
                    Err(e) => {
                        upload_project_metric.success = false;
                        append_error(&mut upload_project_metric.error, format!("代理项目上传失败: {}", e));
                    }
                }
            } else {
                upload_project_metric.success = false;
                append_error(&mut upload_project_metric.error, "代理项目上传跳过：代理 client 未初始化".to_string());
            }
        }

        if test_direct {
            let direct_client = direct_client.as_ref().ok_or_else(|| "直连项目上传跳过：直连 client 未初始化".to_string())?;
            match upload_project_for_speed_test(
                direct_client,
                &base_url,
                &token,
                &project_root_path,
                pfs,
                batch_size,
                max_lines_per_blob,
                project_upload_max_files_limit,
            )
            .await
            {
                Ok(r) => {
                    upload_project_metric.direct_time_ms = Some(r.elapsed_ms);
                    if detail.is_none() {
                        detail = Some(r);
                    }
                }
                Err(e) => {
                    upload_project_metric.success = false;
                    append_error(&mut upload_project_metric.error, format!("直连项目上传失败: {}", e));
                }
            }
        }

        if let Some(r) = detail {
            let mode_label = match project_upload_mode.as_str() {
                "full" => format!("全量 {} 文件", r.planned_files),
                _ => format!("采样 {}/{} 文件", r.planned_files, r.total_files),
            };

            upload_project_metric.name = format!(
                "📦 项目上传 ({}，{}，{} blobs)",
                mode_label,
                format_bytes(r.total_bytes),
                r.blob_count
            );

            if r.skipped_files > 0 {
                upload_project_metric.success = false;
                append_error(&mut upload_project_metric.error, format!("读取失败文件: {} 个", r.skipped_files));
                if let Some(e) = r.first_error {
                    append_error(&mut upload_project_metric.error, e);
                }
            }

            if r.truncated {
                append_error(&mut upload_project_metric.error, "已按采样上限截断文件数量".to_string());
            }
        }
    }
    metrics.push(upload_project_metric);
    
    // 阶段4: 项目上传完成
    emit_progress(4, "项目上传", 90, SpeedTestStageStatus::Completed, 
                  Some(&format!("代理: {}ms, 直连: {}ms", 
                               metrics.last().and_then(|m| m.proxy_time_ms).unwrap_or(0),
                               metrics.last().and_then(|m| m.direct_time_ms).unwrap_or(0))), 
                  None);
    
    // 阶段5: 生成报告
    emit_progress(5, "生成报告", 95, SpeedTestStageStatus::Running, 
                  Some("正在生成诊断报告..."), None);
    
    log::info!("📊 [SpeedTest] === 测试完成，生成报告 ===");
    log::info!("📊 [SpeedTest] 总指标数: {}", metrics.len());
    
    // 生成推荐建议（附带成功率与失败摘要）
    let mut recommendation = generate_recommendation(&metrics, &test_mode);
    let all_success = metrics.iter().all(|m| m.success);

    let total = metrics.len().max(1);
    let ok = metrics.iter().filter(|m| m.success).count();
    recommendation = format!("{} | 总体成功率: {}/{}", recommendation, ok, total);

    if test_proxy {
        let ok_proxy = metrics.iter().filter(|m| m.proxy_time_ms.is_some()).count();
        recommendation = format!("{} | 代理成功: {}/{}", recommendation, ok_proxy, total);
    }

    if test_direct {
        let ok_direct = metrics.iter().filter(|m| m.direct_time_ms.is_some()).count();
        recommendation = format!("{} | 直连成功: {}/{}", recommendation, ok_direct, total);
    }

    if !all_success {
        if let Some(first_fail) = metrics.iter().find(|m| !m.success) {
            if let Some(err) = &first_fail.error {
                let mut err_short = err.replace('\n', " / ");
                if err_short.len() > 120 {
                    err_short.truncate(120);
                    err_short.push_str("...");
                }
                recommendation = format!("{} | 失败示例: {} - {}", recommendation, first_fail.name, err_short);
            } else {
                recommendation = format!("{} | 存在失败项", recommendation);
            }
        }
    }
    
    let result = ProxySpeedTestResult {
        mode: test_mode,
        proxy_info,
        metrics,
        timestamp: chrono::Utc::now().to_rfc3339(),
        recommendation,
        success: all_success,
    };
    
    log::info!("✅ [SpeedTest] 代理测速完成: success={}, metrics={}, recommendation={}", 
               all_success, result.metrics.len(), result.recommendation);
    
    // 输出每个指标的详细结果
    for (i, m) in result.metrics.iter().enumerate() {
        log::debug!("📈 [SpeedTest] 指标[{}] {}: proxy={}ms, direct={}ms, success={}",
                   i, m.name,
                   m.proxy_time_ms.map_or("-".to_string(), |v| v.to_string()),
                   m.direct_time_ms.map_or("-".to_string(), |v| v.to_string()),
                   m.success);
    }
    
    // 阶段5: 全部完成
    let final_status = if all_success { SpeedTestStageStatus::Completed } else { SpeedTestStageStatus::Failed };
    emit_progress(5, "生成报告", 100, final_status, 
                  Some(&format!("成功率: {}/{}", ok, total)), None);
    
    Ok(result)
}

/// 代理设置（用于测速等临时请求）
/// 支持：HTTP / HTTPS / SOCKS5 代理 + Basic Auth
#[derive(Debug, Clone)]
struct ProxySettings {
    proxy_type: String,           // "http" | "https" | "socks5"
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
}

impl ProxySettings {
    /// 构建 reqwest 代理配置（避免泄露密码到日志）
    fn to_reqwest_proxy(&self) -> Result<reqwest::Proxy, String> {
        // 校验代理类型，避免拼接出无效 URL
        match self.proxy_type.as_str() {
            "http" | "https" | "socks5" => {}
            other => return Err(format!("不支持的代理类型: {}（仅支持 http/https/socks5）", other)),
        }

        if self.host.trim().is_empty() {
            return Err("代理主机不能为空".to_string());
        }

        let proxy_url = format!("{}://{}:{}", self.proxy_type, self.host.trim(), self.port);
        let mut reqwest_proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| format!("创建代理失败: {}", e))?;

        // 代理认证（Basic Auth）
        if let Some(username) = self.username.as_deref() {
            let username = username.trim();
            if !username.is_empty() {
                let password = self.password.as_deref().unwrap_or("");
                reqwest_proxy = reqwest_proxy.basic_auth(username, password);
            }
        }

        Ok(reqwest_proxy)
    }
}

/// 上传用的 blob 结构（与 /batch-upload 接口的输入保持一致）
#[derive(Debug, Clone, serde::Serialize)]
struct UploadBlob {
    path: String,
    content: String,
}

/// 读取文件内容，支持多种编码检测（与 acemcp::mcp.rs 保持一致）
fn read_file_with_encoding_for_speed_test(path: &std::path::Path) -> Result<String, String> {
    use std::fs;
    use std::io::Read;

    use encoding_rs::{GBK, WINDOWS_1252, UTF_8};

    let mut file = fs::File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| format!("读取文件失败: {}", e))?;

    // 尝试 utf-8
    let (decoded, _, had_errors) = UTF_8.decode(&buf);
    if !had_errors {
        return Ok(decoded.into_owned());
    }

    // 尝试 gbk
    let (decoded, _, had_errors) = GBK.decode(&buf);
    if !had_errors {
        log::debug!("测速读取文件：成功使用 GBK 编码: {:?}", path);
        return Ok(decoded.into_owned());
    }

    // 尝试 latin-1 (WINDOWS_1252 是 ISO-8859-1 的超集)
    let (decoded, _, had_errors) = WINDOWS_1252.decode(&buf);
    if !had_errors {
        log::debug!("测速读取文件：成功使用 WINDOWS_1252 编码: {:?}", path);
        return Ok(decoded.into_owned());
    }

    // 降级：utf-8 lossy
    let (decoded, _, _) = UTF_8.decode(&buf);
    log::debug!("测速读取文件：使用 UTF-8 (lossy)，部分字符可能丢失: {:?}", path);
    Ok(decoded.into_owned())
}

/// 分割文件内容为多个 blob（如果超过最大行数）
/// 与 acemcp::mcp.rs 保持一致：chunk 索引从 1 开始
fn split_content_for_speed_test(path: &str, content: &str, max_lines: usize) -> Vec<UploadBlob> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let total_lines = lines.len();

    if total_lines <= max_lines {
        return vec![UploadBlob {
            path: path.to_string(),
            content: content.to_string(),
        }];
    }

    let num_chunks = total_lines.div_ceil(max_lines);
    let mut blobs = Vec::new();

    for chunk_idx in 0..num_chunks {
        let start_line = chunk_idx * max_lines;
        let end_line = usize::min(start_line + max_lines, total_lines);
        let chunk_lines = &lines[start_line..end_line];
        let chunk_content = chunk_lines.join("");

        let chunk_path = format!("{}#chunk{}of{}", path, chunk_idx + 1, num_chunks);
        blobs.push(UploadBlob {
            path: chunk_path,
            content: chunk_content,
        });
    }

    blobs
}

/// 构建测速用 HTTP Client（支持代理 + connect_timeout）
/// 说明：测速过程中会多次请求，如果每次都 build client 会有额外开销
fn build_speed_test_client(proxy: Option<&ProxySettings>, timeout_secs: u64) -> Result<reqwest::Client, String> {
    let mut client_builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(crate::constants::network::CONNECTION_TIMEOUT_MS))
        .timeout(std::time::Duration::from_secs(timeout_secs));

    if let Some(p) = proxy {
        client_builder = client_builder.proxy(p.to_reqwest_proxy()?);
    }

    client_builder
        .build()
        .map_err(|e| format!("构建客户端失败: {}", e))
}

/// 上传一批 blobs，返回耗时（毫秒）
async fn upload_blobs_batch(
    client: &reqwest::Client,
    upload_url: &str,
    token: &str,
    blobs: &[UploadBlob],
    timeout_secs: u64,
) -> Result<u64, String> {
    if blobs.is_empty() {
        return Ok(0);
    }

    let payload = serde_json::json!({ "blobs": blobs });
    let start = std::time::Instant::now();

    let resp = client
        .post(upload_url)
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("上传请求失败: {}", e))?;

    let elapsed = start.elapsed().as_millis() as u64;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {} {}", status, body));
    }

    Ok(elapsed)
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        return format!("{:.2}GB", bytes as f64 / GB as f64);
    }
    if bytes >= MB {
        return format!("{:.2}MB", bytes as f64 / MB as f64);
    }
    if bytes >= KB {
        return format!("{:.2}KB", bytes as f64 / KB as f64);
    }
    format!("{}B", bytes)
}

fn append_error(dst: &mut Option<String>, msg: String) {
    match dst {
        Some(s) => {
            s.push('\n');
            s.push_str(&msg);
        }
        None => {
            *dst = Some(msg);
        }
    }
}

struct ProjectUploadResult {
    elapsed_ms: u64,
    planned_files: usize,
    total_files: usize,
    skipped_files: usize,
    blob_count: usize,
    total_bytes: u64,
    truncated: bool,
    first_error: Option<String>,
}

/// 项目上传测速：按文件列表读取内容并批量上传 blobs
/// - `max_files`: Some(n) 表示最多测试 n 个文件（采样），None 表示全量
#[allow(clippy::too_many_arguments)]
async fn upload_project_for_speed_test(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    project_root_path: &str,
    project_files_status: &ProjectFilesStatus,
    batch_size: usize,
    max_lines_per_blob: usize,
    max_files: Option<usize>,
) -> Result<ProjectUploadResult, String> {
    use std::path::PathBuf;

    let total_files = project_files_status.files.len();
    let files_to_test = match max_files {
        Some(max) => usize::min(max, total_files),
        None => total_files,
    };

    let truncated = max_files.is_some() && total_files > files_to_test;
    let upload_url = format!("{}/batch-upload", base_url);

    let start = std::time::Instant::now();

    let mut batch: Vec<UploadBlob> = Vec::with_capacity(batch_size);
    let mut skipped_files = 0usize;
    let mut blob_count = 0usize;
    let mut total_bytes = 0u64;
    let mut first_error: Option<String> = None;

    for file in project_files_status.files.iter().take(files_to_test) {
        let abs_path = PathBuf::from(project_root_path).join(&file.path);

        // 统计文件大小（即使读取失败也尽量统计）
        if let Ok(meta) = std::fs::metadata(&abs_path) {
            total_bytes += meta.len();
        }

        let content = match read_file_with_encoding_for_speed_test(&abs_path) {
            Ok(c) => c,
            Err(e) => {
                skipped_files += 1;
                if first_error.is_none() {
                    first_error = Some(format!("读取文件失败: path={}, error={}", file.path, e));
                }
                continue;
            }
        };

        let blobs = split_content_for_speed_test(&file.path, &content, max_lines_per_blob);
        blob_count += blobs.len();

        for b in blobs {
            batch.push(b);
            if batch.len() >= batch_size {
                // 上传一批
                let _ = upload_blobs_batch(client, &upload_url, token, &batch, 120).await?;
                batch.clear();
            }
        }
    }

    if !batch.is_empty() {
        let _ = upload_blobs_batch(client, &upload_url, token, &batch, 120).await?;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;

    Ok(ProjectUploadResult {
        elapsed_ms,
        planned_files: files_to_test,
        total_files,
        skipped_files,
        blob_count,
        total_bytes,
        truncated,
        first_error,
    })
}

fn build_single_file_blobs_for_speed_test(
    project_root_path: &str,
    rel_path: &str,
    max_lines_per_blob: usize,
) -> Result<(Vec<UploadBlob>, u64), String> {
    use std::path::PathBuf;

    let abs_path = PathBuf::from(project_root_path).join(rel_path);
    let file_bytes = std::fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0);

    let content = read_file_with_encoding_for_speed_test(&abs_path)
        .map_err(|e| format!("读取文件失败: path={}, error={}", rel_path, e))?;

    let blobs = split_content_for_speed_test(rel_path, &content, max_lines_per_blob);
    Ok((blobs, file_bytes))
}

/// Ping 测试辅助函数
/// 注意：使用 GET 方法而非 HEAD，因为部分 ACE 服务器的 /health 端点不支持 HEAD 方法（返回 405）
async fn ping_endpoint(client: &reqwest::Client, url: &str, token: &str) -> Result<u64, String> {
    log::debug!("🔗 [Ping] 开始请求: url={}", url);
    
    let start = std::time::Instant::now();
    let response = client
        .get(url)  // 使用 GET 方法代替 HEAD，解决 HTTP 405 Method Not Allowed 问题
        .timeout(std::time::Duration::from_secs(10))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| {
            log::warn!("❌ [Ping] 请求失败: url={}, error={}", url, e);
            format!("请求失败: {}", e)
        })?;
    
    let elapsed = start.elapsed().as_millis() as u64;
    let status = response.status();
    
    log::debug!("✅ [Ping] 响应: url={}, status={}, elapsed={}ms", url, status, elapsed);
    
    if status.is_success() || status.as_u16() == 404 {
        // 404 也算成功，因为只是测试连通性
        // 2xx 成功响应 或 404 表示端点存在但资源不存在，连通性正常
        Ok(elapsed)
    } else {
        log::warn!("⚠️ [Ping] HTTP 错误响应: url={}, status={}", url, status);
        Err(format!("HTTP {}", status))
    }
}

/// 搜索测试结果（包含耗时和结果预览）
struct SearchEndpointResult {
    elapsed_ms: u64,
    preview: Option<super::types::SearchResultPreview>,
}

/// 搜索测试辅助函数
/// 返回耗时和搜索结果预览（用于前端展示）
async fn search_endpoint(client: &reqwest::Client, url: &str, token: &str, payload: &serde_json::Value) -> Result<SearchEndpointResult, String> {
    let start = std::time::Instant::now();
    let response = client
        .post(url)
        .timeout(std::time::Duration::from_secs(30))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    
    let elapsed = start.elapsed().as_millis() as u64;
    
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    
    // 解析响应内容，提取搜索结果预览
    let body = response.text().await.unwrap_or_default();
    
    // 输出原始响应内容用于调试（截断显示）
    let body_preview = if body.len() > 500 {
        format!("{}... (total {}B)", &body[..500], body.len())
    } else {
        body.clone()
    };
    log::debug!("🔍 [SpeedTest] 搜索原始响应: {}", body_preview);
    
    let preview = parse_search_result_preview(&body);
    
    Ok(SearchEndpointResult {
        elapsed_ms: elapsed,
        preview,
    })
}

/// 解析搜索结果响应，提取预览片段
fn parse_search_result_preview(body: &str) -> Option<super::types::SearchResultPreview> {
    use super::types::{SearchResultPreview, SearchResultSnippet};
    
    let response_length = body.len();
    
    // 尝试解析 JSON 响应
    let json: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("⚠️ [SpeedTest] 搜索响应不是有效 JSON: error={}, body={}", e, 
                      if body.len() > 100 { &body[..100] } else { body });
            // 如果不是 JSON，返回基本信息
            return Some(SearchResultPreview {
                total_matches: 0,
                snippets: vec![],
                response_length,
            });
        }
    };
    
    // 输出 JSON 顶层键用于调试
    if let Some(obj) = json.as_object() {
        let keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
        log::debug!("🔍 [SpeedTest] JSON 顶层键: {:?}", keys);
    }
    
    // ACE API 返回的搜索结果在 formatted_retrieval 字段中（字符串格式）
    let mut snippets = Vec::new();
    let mut total_matches = 0;
    
    // 优先检查 formatted_retrieval 字段（ACE API 的标准搜索结果字段）
    if let Some(formatted) = json.get("formatted_retrieval").and_then(|v| v.as_str()) {
        log::debug!("🔍 [SpeedTest] 发现 formatted_retrieval 字段, 长度={}", formatted.len());
        
        if !formatted.is_empty() && formatted != "No relevant code context found for your query." {
            // 跳过 ACE 标题行（如 "The following code sections were retrieved:"）
            let content = formatted
                .strip_prefix("The following code sections were retrieved:")
                .or_else(|| formatted.strip_prefix("The following code sections were retrieved:\n"))
                .unwrap_or(formatted)
                .trim();
            
            log::debug!("🔍 [SpeedTest] 处理后内容长度={}", content.len());
            
            // ACE 格式通常是按 "---" 分隔的多个代码块
            // 每个块包含文件路径和代码内容
            let blocks: Vec<&str> = content
                .split("\n---\n")
                .filter(|b| !b.trim().is_empty() && b.len() > 10)
                .collect();
            
            // 如果没有 --- 分隔，尝试按双空行分隔
            let blocks = if blocks.len() <= 1 {
                content
                    .split("\n\n")
                    .filter(|b| !b.trim().is_empty() && b.len() > 10)
                    .collect()
            } else {
                blocks
            };
            
            total_matches = blocks.len().max(1); // 至少有一个匹配
            log::debug!("🔍 [SpeedTest] 分割出 {} 个代码块", blocks.len());
            
            for block in blocks.iter().take(5) {
                let lines: Vec<&str> = block.lines().collect();
                if lines.is_empty() {
                    continue;
                }
                
                // 尝试从第一行提取文件路径
                // ACE 格式可能是 "Path: xxx" 或 "File: xxx" 或直接是路径
                let first_line = lines.first().unwrap_or(&"");
                let file_path = first_line
                    .strip_prefix("Path: ")
                    .or_else(|| first_line.strip_prefix("File: "))
                    .or_else(|| first_line.strip_prefix("# "))
                    .or_else(|| first_line.strip_prefix("## "))
                    .or_else(|| {
                        // 如果第一行看起来是文件路径（包含 / 或 \ 或常见扩展名）
                        if first_line.contains('/') || first_line.contains('\\') 
                           || first_line.ends_with(".rs") || first_line.ends_with(".ts")
                           || first_line.ends_with(".vue") || first_line.ends_with(".py") {
                            Some(*first_line)
                        } else {
                            None
                        }
                    })
                    .unwrap_or("代码片段")
                    .trim()
                    .to_string();
                
                // 提取代码片段（去除路径行，取前20行）
                let snippet: String = lines.iter()
                    .skip(1)
                    .take(20)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n");
                
                let snippet_content = if snippet.is_empty() || snippet.len() < 10 {
                    // 如果没有内容，使用整个块（可能第一行不是路径）
                    lines.iter().take(20).copied().collect::<Vec<_>>().join("\n")
                } else {
                    snippet
                };
                
                // 跳过只有标题的块
                if snippet_content.trim().is_empty() 
                   || snippet_content.starts_with("The following") {
                    continue;
                }
                
                snippets.push(SearchResultSnippet {
                    file_path,
                    snippet: if snippet_content.len() > 800 {
                        format!("{}...", &snippet_content[..800])
                    } else {
                        snippet_content
                    },
                    line_number: None,
                });
            }
        }
        
        // 如果成功解析了 formatted_retrieval，直接返回
        if total_matches > 0 || !snippets.is_empty() {
            log::info!("🔍 [SpeedTest] 从 formatted_retrieval 解析: matches={}, snippets={}", 
                      total_matches, snippets.len());
            return Some(SearchResultPreview {
                total_matches,
                snippets,
                response_length,
            });
        }
    }
    
    // 回退：尝试从其他字段提取（兼容其他 API 格式）
    
    // 尝试从不同的 JSON 结构中提取结果
    if let Some(content) = json.get("content") {
        // 如果是字符串类型的内容，提取代码片段
        if let Some(text) = content.as_str() {
            total_matches = 1;
            // 提取文件路径和代码片段（ACE 格式通常是 "Path: xxx\n内容"）
            for block in text.split("\n\n").take(3) {
                let lines: Vec<&str> = block.lines().collect();
                if lines.is_empty() {
                    continue;
                }
                
                // 尝试提取文件路径
                let file_path = lines.first()
                    .and_then(|l| l.strip_prefix("Path: "))
                    .unwrap_or("unknown")
                    .to_string();
                
                // 提取代码片段（去除路径行，最多5行）
                let snippet: String = lines.iter()
                    .skip(1)
                    .take(5)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n");
                
                if !snippet.is_empty() {
                    snippets.push(SearchResultSnippet {
                        file_path,
                        snippet: if snippet.len() > 200 {
                            format!("{}...", &snippet[..200])
                        } else {
                            snippet
                        },
                        line_number: None,
                    });
                }
            }
        }
        // 如果是数组类型
        else if let Some(arr) = content.as_array() {
            total_matches = arr.len();
            for item in arr.iter().take(3) {
                if let Some(text_item) = item.get("text").and_then(|t| t.as_str()) {
                    // 尝试从文本中提取文件路径
                    let file_path = text_item.lines()
                        .find(|l| l.starts_with("Path: ") || l.contains(".rs") || l.contains(".ts"))
                        .unwrap_or("unknown")
                        .to_string();
                    
                    let snippet = text_item.lines()
                        .take(5)
                        .collect::<Vec<_>>()
                        .join("\n");
                    
                    snippets.push(SearchResultSnippet {
                        file_path,
                        snippet: if snippet.len() > 200 {
                            format!("{}...", &snippet[..200])
                        } else {
                            snippet
                        },
                        line_number: None,
                    });
                }
            }
        }
    }
    
    // 尝试从 results 字段提取
    if snippets.is_empty() {
        if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
            total_matches = results.len();
            for result in results.iter().take(3) {
                let file_path = result.get("path")
                    .or_else(|| result.get("file"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let snippet = result.get("content")
                    .or_else(|| result.get("snippet"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                
                let line_number = result.get("line")
                    .or_else(|| result.get("line_number"))
                    .and_then(|l| l.as_u64())
                    .map(|l| l as u32);
                
                if !snippet.is_empty() {
                    snippets.push(SearchResultSnippet {
                        file_path,
                        snippet: if snippet.len() > 200 {
                            format!("{}...", &snippet[..200])
                        } else {
                            snippet
                        },
                        line_number,
                    });
                }
            }
        }
    }
    
    Some(SearchResultPreview {
        total_matches,
        snippets,
        response_length,
    })
}

/// 生成推荐建议
fn generate_recommendation(metrics: &[SpeedTestMetric], mode: &str) -> String {
    if mode != "compare" {
        return "单模式测试完成".to_string();
    }
    
    let mut proxy_total: u64 = 0;
    let mut direct_total: u64 = 0;
    let mut proxy_count = 0;
    let mut direct_count = 0;
    
    for m in metrics {
        if let Some(pt) = m.proxy_time_ms {
            proxy_total += pt;
            proxy_count += 1;
        }
        if let Some(dt) = m.direct_time_ms {
            direct_total += dt;
            direct_count += 1;
        }
    }
    
    if proxy_count == 0 || direct_count == 0 {
        return "无法对比，部分测试失败".to_string();
    }
    
    let proxy_avg = proxy_total / proxy_count as u64;
    let direct_avg = direct_total / direct_count as u64;
    
    if proxy_avg < direct_avg {
        let improvement = ((direct_avg - proxy_avg) as f64 / direct_avg as f64 * 100.0) as u32;
        format!("🟢 建议启用代理，性能提升约 {}%", improvement)
    } else if direct_avg < proxy_avg {
        let degradation = ((proxy_avg - direct_avg) as f64 / proxy_avg as f64 * 100.0) as u32;
        format!("🔴 建议直连，代理性能下降约 {}%", degradation)
    } else {
        "🟡 代理与直连性能相当".to_string()
    }
}
