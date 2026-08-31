use crate::config::{load_config_and_apply_window_settings, AppState};
use crate::log_important;
use crate::ui::exit_handler::setup_exit_handlers;
use crate::ui::{initialize_audio_asset_manager, setup_window_event_listeners};
use tauri::{AppHandle, Manager};

/// 应用设置和初始化
pub async fn setup_application(app_handle: &AppHandle) -> Result<(), String> {
    // 中文说明：把索引任务事件接入 GUI；MCP 独立进程仍会通过 manifest 持久化事件。
    crate::mcp::tools::acemcp::jobs::register_event_app(app_handle);
    let state = app_handle.state::<AppState>();

    // 加载配置并应用窗口设置
    if let Err(e) = load_config_and_apply_window_settings(&state, app_handle).await {
        log_important!(warn, "加载配置失败: {}", e);
    }

    // 中文说明：GUI 启动不等待索引恢复；任务由持久化检查点后台接管，避免阻塞首屏。
    tauri::async_runtime::spawn(async {
        if let Err(error) = crate::mcp::tools::acemcp::mcp::resume_index_jobs().await {
            log_important!(warn, "GUI 启动恢复 ACE 未完成索引任务失败: {}", error);
        }
    });

    // 初始化音频资源管理器
    if let Err(e) = initialize_audio_asset_manager(app_handle) {
        log_important!(warn, "初始化音频资源管理器失败: {}", e);
    }

    // 设置窗口事件监听器
    setup_window_event_listeners(app_handle);

    // 设置退出处理器
    if let Err(e) = setup_exit_handlers(app_handle) {
        log_important!(warn, "设置退出处理器失败: {}", e);
    }

    // 中文说明：应用启动后延迟检查 GitHub 代理缓存，仅在 24 小时 TTL 过期时测速。
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        if let Err(e) = crate::network::refresh_github_proxy_cache().await {
            log_important!(warn, "刷新 GitHub 代理站缓存失败: {}", e);
        }
    });

    Ok(())
}
