use tauri::{AppHandle, State};

use crate::config::{AppState, save_config};
use super::AcemcpTool;
use super::types::{AcemcpRequest, ProjectIndexStatus, ProjectsIndexStatus, ProjectFilesStatus};
use reqwest;

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
    let (effective_base_url, effective_token) = {
        let config = state.config
            .lock()
            .map_err(|e| format!("获取配置失败: {}", e))?;
        
        let base_url = config.mcp_config.acemcp_base_url.as_ref().unwrap_or(&args.base_url).clone();
        let token = config.mcp_config.acemcp_token.as_ref().unwrap_or(&args.token).clone();
        (base_url, token)
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
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

/// 读取日志文件内容
#[tauri::command]
pub async fn read_acemcp_logs(_state: State<'_, AppState>) -> Result<Vec<String>, String> {
    // 使用 dirs::config_dir() 获取系统配置目录，确保跨平台兼容性
    // Windows: C:\Users\<用户>\AppData\Roaming\sanshu\log\acemcp.log
    // Linux: ~/.config/sanshu/log/acemcp.log
    // macOS: ~/Library/Application Support/sanshu/log/acemcp.log
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "无法获取系统配置目录，请检查操作系统环境".to_string())?;

    let log_path = config_dir.join("sanshu").join("log").join("acemcp.log");

    // 确保日志目录存在
    if let Some(log_dir) = log_path.parent() {
        if !log_dir.exists() {
            std::fs::create_dir_all(log_dir)
                .map_err(|e| format!("创建日志目录失败: {} (路径: {})", e, log_dir.display()))?;
        }
    }

    // 如果日志文件不存在，返回空数组
    if !log_path.exists() {
        return Ok(vec![]);
    }

    // 读取日志文件内容
    let content = std::fs::read_to_string(&log_path)
        .map_err(|e| format!("读取日志文件失败: {} (路径: {})", e, log_path.display()))?;

    // 返回最近1000行日志
    let all_lines: Vec<String> = content
        .lines()
        .map(|s| s.to_string())
        .collect();

    // 只返回最后1000行
    let lines: Vec<String> = if all_lines.len() > 1000 {
        let skip_count = all_lines.len() - 1000;
        all_lines.into_iter().skip(skip_count).collect()
    } else {
        all_lines
    };

    Ok(lines)
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
    })
}

#[derive(Debug, serde::Serialize)]
pub struct DebugSearchResult {
    pub success: bool,
    pub result: Option<String>,
    pub error: Option<String>,
}

/// 纯 Rust 的调试命令：直接执行 acemcp 搜索，返回结果
#[tauri::command]
pub async fn debug_acemcp_search(
    project_root_path: String,
    query: String,
    _app: AppHandle,
) -> Result<DebugSearchResult, String> {
    let req = AcemcpRequest { project_root_path, query };
    
    // 调用搜索函数（日志会通过 log crate 输出到 stderr）
    let search_result = AcemcpTool::search_context(req).await;
    
    match search_result {
        Ok(result) => {
            let mut result_text = String::new();
            if let Ok(val) = serde_json::to_value(&result) {
                if let Some(arr) = val.get("content").and_then(|v| v.as_array()) {
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
            })
        }
        Err(e) => {
            Ok(DebugSearchResult {
                success: false,
                result: None,
                error: Some(format!("执行失败: {}", e)),
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
    Ok(AcemcpTool::get_all_index_status())
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

/// 手动触发索引更新
#[tauri::command]
pub async fn trigger_acemcp_index_update(project_root_path: String) -> Result<String, String> {
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
pub fn set_auto_index_enabled(enabled: bool) -> Result<(), String> {
    let watcher_manager = super::watcher::get_watcher_manager();
    watcher_manager.set_auto_index_enabled(enabled);
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

/// 删除指定项目的索引记录
/// 同时清理 projects.json 和 projects_status.json 中的数据
#[tauri::command]
pub async fn remove_acemcp_project_index(project_root_path: String) -> Result<String, String> {
    use std::path::PathBuf;
    use std::fs;
    use std::collections::HashMap;

    // 辅助函数：规范化路径 key（去除扩展路径前缀，统一使用正斜杠）
    fn normalize_path_key(path: &str) -> String {
        let mut normalized = path.to_string();
        // 去除 Windows 扩展长度路径前缀
        if normalized.starts_with("\\\\?\\") {
            normalized = normalized[4..].to_string();
        } else if normalized.starts_with("//?/") {
            normalized = normalized[4..].to_string();
        }
        // 统一使用正斜杠
        normalized.replace('\\', "/")
    }

    // 规范化传入的路径
    let normalized_root = normalize_path_key(&project_root_path);

    log::info!("[remove_acemcp_project_index] 开始删除项目索引记录");
    log::info!("[remove_acemcp_project_index] 原始路径: {}", project_root_path);
    log::info!("[remove_acemcp_project_index] 规范化后路径: {}", normalized_root);

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = home.join(".acemcp").join("data");

    let mut projects_deleted = false;
    let mut status_deleted = false;

    // 1. 从 projects.json 中删除项目的 blob 列表
    let projects_path = data_dir.join("projects.json");
    if projects_path.exists() {
        if let Ok(data) = fs::read_to_string(&projects_path) {
            if let Ok(mut projects) = serde_json::from_str::<HashMap<String, Vec<String>>>(&data) {
                // 调试日志：输出现有的 key 列表
                let existing_keys: Vec<&String> = projects.keys().collect();
                log::info!("[remove_acemcp_project_index] projects.json 中现有项目: {:?}", existing_keys);
                
                // 遍历查找匹配的 key（对每个 key 也进行规范化后比较）
                let key_to_remove: Option<String> = projects.keys()
                    .find(|k| normalize_path_key(k) == normalized_root)
                    .cloned();
                
                if let Some(key) = key_to_remove {
                    log::info!("[remove_acemcp_project_index] 找到匹配的 key: {}", key);
                    projects.remove(&key);
                    if let Ok(new_data) = serde_json::to_string_pretty(&projects) {
                        let _ = fs::write(&projects_path, new_data);
                        log::info!("[remove_acemcp_project_index] ✓ 已从 projects.json 删除项目: {}", key);
                        projects_deleted = true;
                    }
                } else {
                    log::warn!("[remove_acemcp_project_index] ✗ 在 projects.json 中未找到匹配的项目，规范化路径: {}", normalized_root);
                }
            }
        }
    } else {
        log::warn!("[remove_acemcp_project_index] projects.json 文件不存在: {:?}", projects_path);
    }

    // 2. 从 projects_status.json 中删除项目状态
    let status_path = data_dir.join("projects_status.json");
    if status_path.exists() {
        if let Ok(data) = fs::read_to_string(&status_path) {
            if let Ok(mut status) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(projects) = status.get_mut("projects") {
                    if let Some(map) = projects.as_object_mut() {
                        // 调试日志：输出现有的 key 列表
                        let existing_keys: Vec<&String> = map.keys().collect();
                        log::info!("[remove_acemcp_project_index] projects_status.json 中现有项目: {:?}", existing_keys);
                        
                        // 遍历查找匹配的 key（对每个 key 也进行规范化后比较）
                        let key_to_remove: Option<String> = map.keys()
                            .find(|k| normalize_path_key(k) == normalized_root)
                            .cloned();
                        
                        if let Some(key) = key_to_remove {
                            log::info!("[remove_acemcp_project_index] 找到匹配的 key: {}", key);
                            map.remove(&key);
                            if let Ok(new_data) = serde_json::to_string_pretty(&status) {
                                let _ = fs::write(&status_path, new_data);
                                log::info!("[remove_acemcp_project_index] ✓ 已从 projects_status.json 删除项目: {}", key);
                                status_deleted = true;
                            }
                        } else {
                            log::warn!("[remove_acemcp_project_index] ✗ 在 projects_status.json 中未找到匹配的项目，规范化路径: {}", normalized_root);
                        }
                    }
                }
            }
        }
    } else {
        log::warn!("[remove_acemcp_project_index] projects_status.json 文件不存在: {:?}", status_path);
    }

    // 3. 停止该项目的文件监听（如果有）
    let watcher_manager = super::watcher::get_watcher_manager();
    let _ = watcher_manager.stop_watching(&normalized_root);

    // 汇总删除结果
    if projects_deleted || status_deleted {
        log::info!("[remove_acemcp_project_index] 删除完成: projects.json={}, status.json={}", projects_deleted, status_deleted);
        Ok(format!("已删除项目索引记录: {}", normalized_root))
    } else {
        log::warn!("[remove_acemcp_project_index] 未能从任何文件中删除项目，可能路径不匹配");
        // 仍返回成功，因为可能项目本身就不存在（已被其他方式删除）
        Ok(format!("项目索引记录可能已不存在: {}", normalized_root))
    }
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

