// Tauri 命令入口
// 将提示词增强功能暴露给前端调用

use tauri::{AppHandle, Emitter};
use super::types::*;
use super::core::PromptEnhancer;
use super::history::ChatHistoryManager;
use crate::log_important;

/// 流式增强提示词（主要入口）
/// 通过 Tauri Event 推送流式结果给前端
#[tauri::command]
pub async fn enhance_prompt_stream(
    app_handle: AppHandle,
    prompt: String,
    project_root_path: Option<String>,
    current_file_path: Option<String>,
    include_history: Option<bool>,
) -> Result<EnhanceResponse, String> {
    log_important!(info, "收到增强请求: prompt_len={}, project={:?}", 
        prompt.len(), 
        project_root_path.as_ref().map(|p| p.len())
    );

    // 创建增强器
    let mut enhancer = PromptEnhancer::from_acemcp_config()
        .await
        .map_err(|e| format!("初始化增强器失败: {}", e))?;

    if let Some(ref path) = project_root_path {
        enhancer = enhancer.with_project_root(path);
    }

    let request = EnhanceRequest {
        prompt: prompt.clone(),
        project_root_path: project_root_path.clone(),
        current_file_path,
        include_history: include_history.unwrap_or(true),
    };

    // 使用流式增强
    let app = app_handle.clone();
    let result = enhancer.enhance_stream(request, move |event| {
        // 通过 Tauri Event 推送给前端
        if let Err(e) = app.emit("enhance-stream", &event) {
            log_important!(warn, "推送增强事件失败: {}", e);
        }
    }).await;

    match result {
        Ok(response) => {
            // 如果增强成功，记录到对话历史
            if response.success {
                if let Some(ref path) = project_root_path {
                    if let Ok(manager) = ChatHistoryManager::new(path) {
                        let _ = manager.add_entry(
                            &prompt,
                            &response.enhanced_prompt,
                            "enhance"
                        );
                    }
                }
            }
            Ok(response)
        }
        Err(e) => Err(format!("增强失败: {}", e))
    }
}

/// 同步增强提示词（简化版，等待完成后返回）
#[tauri::command]
pub async fn enhance_prompt(
    prompt: String,
    project_root_path: Option<String>,
    current_file_path: Option<String>,
    include_history: Option<bool>,
) -> Result<EnhanceResponse, String> {
    log_important!(info, "收到同步增强请求: prompt_len={}", prompt.len());

    // 创建增强器
    let mut enhancer = PromptEnhancer::from_acemcp_config()
        .await
        .map_err(|e| format!("初始化增强器失败: {}", e))?;

    if let Some(ref path) = project_root_path {
        enhancer = enhancer.with_project_root(path);
    }

    let request = EnhanceRequest {
        prompt: prompt.clone(),
        project_root_path: project_root_path.clone(),
        current_file_path,
        include_history: include_history.unwrap_or(true),
    };

    enhancer.enhance(request)
        .await
        .map_err(|e| format!("增强失败: {}", e))
}

/// 添加对话历史记录
#[tauri::command]
pub async fn add_chat_history(
    project_root_path: String,
    user_input: String,
    ai_response: String,
    source: Option<String>,
) -> Result<String, String> {
    let manager = ChatHistoryManager::new(&project_root_path)
        .map_err(|e| format!("创建历史管理器失败: {}", e))?;
    
    manager.add_entry(
        &user_input,
        &ai_response,
        &source.unwrap_or_else(|| "popup".to_string())
    ).map_err(|e| format!("添加历史记录失败: {}", e))
}

/// 获取对话历史
#[tauri::command]
pub async fn get_chat_history(
    project_root_path: String,
    count: Option<usize>,
) -> Result<Vec<super::history::ChatEntry>, String> {
    let manager = ChatHistoryManager::new(&project_root_path)
        .map_err(|e| format!("创建历史管理器失败: {}", e))?;
    
    Ok(manager.get_recent(count.unwrap_or(20)))
}

/// 清空对话历史
#[tauri::command]
pub async fn clear_chat_history(
    project_root_path: String,
) -> Result<(), String> {
    let manager = ChatHistoryManager::new(&project_root_path)
        .map_err(|e| format!("创建历史管理器失败: {}", e))?;
    
    manager.clear()
        .map_err(|e| format!("清空历史失败: {}", e))
}
