// Tavily Tauri 命令入口
// 提供配置读写和连接测试功能

use tauri::State;
use crate::config::AppState;
use super::types::TavilyTestConnectionResponse;

/// 获取 Tavily 配置
#[tauri::command]
pub async fn get_tavily_config(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let config = state.config
        .lock()
        .map_err(|e| format!("获取配置失败: {}", e))?;

    Ok(serde_json::json!({
        "api_key": config.mcp_config.tavily_api_key.as_deref().unwrap_or(""),
    }))
}

/// 保存 Tavily 配置
#[tauri::command]
pub async fn save_tavily_config(
    api_key: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    {
        let mut config = state.config
            .lock()
            .map_err(|e| format!("获取配置失败: {}", e))?;

        // 空字符串视为清除 API Key
        config.mcp_config.tavily_api_key = if api_key.trim().is_empty() {
            None
        } else {
            Some(api_key.trim().to_string())
        };
    }

    crate::config::save_config(&state, &app).await
        .map_err(|e| format!("保存配置失败: {}", e))?;

    Ok(())
}

/// 测试 Tavily 连接
#[tauri::command]
pub async fn test_tavily_connection(
    api_key: String,
) -> Result<TavilyTestConnectionResponse, String> {
    use reqwest::Client;
    use std::time::Duration;

    if api_key.trim().is_empty() {
        return Ok(TavilyTestConnectionResponse {
            success: false,
            message: "API Key 不能为空".to_string(),
            preview: None,
        });
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    // 使用一次最小成本的 basic search 测试
    let body = serde_json::json!({
        "query": "test",
        "max_results": 1,
        "search_depth": "basic",
        "include_usage": true,
    });

    match client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .json(&body)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                let text = response.text().await.unwrap_or_default();
                // 尝试解析获取结果数量
                let preview = if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    let results_count = parsed["results"].as_array().map(|a| a.len()).unwrap_or(0);
                    let response_time = parsed["response_time"].as_f64().unwrap_or(0.0);
                    Some(format!("获取到 {} 条结果，响应时间 {:.2}s", results_count, response_time))
                } else {
                    None
                };

                Ok(TavilyTestConnectionResponse {
                    success: true,
                    message: "连接成功！API Key 有效。".to_string(),
                    preview,
                })
            } else {
                let error_text = response.text().await.unwrap_or_default();
                let msg = match status.as_u16() {
                    401 => "API Key 无效或已过期".to_string(),
                    429 => "请求频率超限，请稍后重试".to_string(),
                    402 => "信用点已耗尽".to_string(),
                    _ => format!("HTTP {} - {}", status.as_u16(), if error_text.len() > 200 { &error_text[..200] } else { &error_text }),
                };

                Ok(TavilyTestConnectionResponse {
                    success: false,
                    message: format!("连接失败: {}", msg),
                    preview: None,
                })
            }
        }
        Err(e) => {
            let msg = if e.is_timeout() {
                "连接超时（15s）".to_string()
            } else if e.is_connect() {
                "无法连接到 Tavily API，请检查网络".to_string()
            } else {
                format!("请求失败: {}", e)
            };

            Ok(TavilyTestConnectionResponse {
                success: false,
                message: msg,
                preview: None,
            })
        }
    }
}
