// 代理配置相关的 Tauri 命令
use tauri::{AppHandle, State};
use crate::config::{AppState, ProxyConfig, save_config};
use crate::{log_important, log_debug};
use super::{ProxyDetector, ProxyInfo, proxy::ProxyType};

/// 获取代理配置
#[tauri::command]
pub async fn get_proxy_config(state: State<'_, AppState>) -> Result<ProxyConfig, String> {
    log_debug!("[network] 获取代理配置");
    
    let config = state
        .config
        .lock()
        .map_err(|e| format!("获取配置失败: {}", e))?;
    
    Ok(config.proxy_config.clone())
}

/// 设置代理配置
#[tauri::command]
pub async fn set_proxy_config(
    proxy_config: ProxyConfig,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    log_important!(info, "[network] 设置代理配置: enabled={}, proxy_type={:?}, host={:?}, port={:?}", 
        proxy_config.enabled, proxy_config.proxy_type, proxy_config.host, proxy_config.port);
    
    {
        let mut config = state
            .config
            .lock()
            .map_err(|e| format!("获取配置失败: {}", e))?;
        config.proxy_config = proxy_config;
    }

    // 保存配置到文件
    save_config(&state, &app)
        .await
        .map_err(|e| format!("保存配置失败: {}", e))?;

    log_debug!("[network] 代理配置已保存");
    Ok(())
}

/// 测试代理连接
#[tauri::command]
pub async fn test_proxy_connection(
    proxy_type: String,
    host: String,
    port: u16,
) -> Result<bool, String> {
    log_important!(info, "[network] 测试代理连接: {}://{}:{}", proxy_type, host, port);
    
    let proxy_type_enum = match proxy_type.as_str() {
        "socks5" => ProxyType::Socks5,
        _ => ProxyType::Http,
    };
    
    let proxy_info = ProxyInfo::new(proxy_type_enum, host.clone(), port);
    
    let is_available = ProxyDetector::check_proxy(&proxy_info).await;
    
    if is_available {
        log_important!(info, "[network] 代理连接测试成功: {}:{}", host, port);
    } else {
        log_important!(warn, "[network] 代理连接测试失败: {}:{}", host, port);
    }
    
    Ok(is_available)
}

/// 自动检测可用代理
#[tauri::command]
pub async fn detect_available_proxy() -> Result<Option<ProxyInfo>, String> {
    log_important!(info, "[network] 开始自动检测可用代理");
    
    let proxy_info = ProxyDetector::detect_available_proxy().await;
    
    if let Some(ref info) = proxy_info {
        log_important!(info, "[network] 检测到可用代理: {}:{} ({})", info.host, info.port, info.proxy_type);
    } else {
        log_important!(info, "[network] 未检测到可用代理");
    }
    
    Ok(proxy_info)
}

