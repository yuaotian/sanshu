use tauri::{State, Manager};
use crate::config::{AppState, save_config};
use crate::constants::{window, validation};
use serde::{Deserialize, Serialize};

/// 在包含指定物理坐标的显示器上居中窗口
/// `logical_width`/`logical_height` 为窗口的逻辑尺寸，用于配合目标显示器缩放计算居中位置
/// 返回 true 表示找到目标显示器并居中成功
pub fn center_on_monitor_containing(
    win: &tauri::WebviewWindow,
    physical_x: i32,
    physical_y: i32,
    logical_width: f64,
    logical_height: f64,
) -> bool {
    let monitors = match win.available_monitors() {
        Ok(m) => m,
        Err(_) => return false,
    };

    log::debug!("可用显示器数量: {}", monitors.len());
    for (i, m) in monitors.iter().enumerate() {
        let pos = m.position();
        let size = m.size();
        log::debug!("  显示器[{}]: 位置({}, {}) 尺寸 {}x{} 缩放 {}", i, pos.x, pos.y, size.width, size.height, m.scale_factor());
    }

    let target = monitors.iter().find(|m| {
        let pos = m.position();
        let size = m.size();
        physical_x >= pos.x
            && physical_x < pos.x + size.width as i32
            && physical_y >= pos.y
            && physical_y < pos.y + size.height as i32
    });

    if let Some(monitor) = target {
        let m_pos = monitor.position();
        let m_size = monitor.size();
        let scale = monitor.scale_factor();
        let win_w = (logical_width * scale) as i32;
        let win_h = (logical_height * scale) as i32;
        let cx = m_pos.x + (m_size.width as i32 - win_w) / 2;
        let cy = m_pos.y + (m_size.height as i32 - win_h) / 2;
        log::debug!("找到目标显示器(缩放{}), 居中到 ({}, {})", scale, cx, cy);

        win.set_position(tauri::PhysicalPosition::new(cx, cy)).is_ok()
    } else {
        log::debug!("坐标 ({}, {}) 不在任何显示器范围内", physical_x, physical_y);
        false
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WindowSizeUpdate {
    pub width: f64,
    pub height: f64,
    pub fixed: bool,
}

#[tauri::command]
pub async fn apply_window_constraints(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    let (window_config, always_on_top) = {
        let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        (config.ui_config.window_config.clone(), config.ui_config.always_on_top)
    };

    if let Some(window) = app.get_webview_window("main") {
        // 设置窗口约束
        if let Err(e) = window.set_min_size(Some(tauri::LogicalSize::new(
            window_config.min_width,
            window_config.min_height,
        ))) {
            return Err(format!("设置最小窗口大小失败: {}", e));
        }

        if let Err(e) = window.set_max_size(Some(tauri::LogicalSize::new(
            window_config.max_width,
            window_config.max_height,
        ))) {
            return Err(format!("设置最大窗口大小失败: {}", e));
        }

        // 如果启用了自动调整大小，设置为合适的初始大小
        if window_config.auto_resize {
            let initial_width = window_config.min_width;
            let initial_height = (window_config.min_height + window_config.max_height) / 2.0;
            
            if let Err(e) = window.set_size(tauri::LogicalSize::new(initial_width, initial_height)) {
                return Err(format!("设置窗口大小失败: {}", e));
            }
        }

        // 确保置顶状态在应用窗口约束后仍然有效
        if let Err(e) = window.set_always_on_top(always_on_top) {
            log::warn!("应用窗口约束后重新设置置顶状态失败: {}", e);
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn update_window_size(size_update: WindowSizeUpdate, state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    // 更新配置
    {
        let mut config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;

        // 更新模式设置
        config.ui_config.window_config.fixed = size_update.fixed;

        // 更新当前模式的尺寸
        config.ui_config.window_config.update_current_size(size_update.width, size_update.height);

        if size_update.fixed {
            // 固定模式：设置最大和最小尺寸为相同值
            config.ui_config.window_config.max_width = size_update.width;
            config.ui_config.window_config.max_height = size_update.height;
            config.ui_config.window_config.min_width = size_update.width;
            config.ui_config.window_config.min_height = size_update.height;
            config.ui_config.window_config.auto_resize = false;
        } else {
            // 自由拉伸模式：设置合理的最小值和限制的最大值
            config.ui_config.window_config.min_width = window::MIN_WIDTH;
            config.ui_config.window_config.min_height = window::MIN_HEIGHT;
            config.ui_config.window_config.max_width = window::MAX_WIDTH;
            config.ui_config.window_config.max_height = window::MAX_HEIGHT;
            config.ui_config.window_config.auto_resize = window::DEFAULT_AUTO_RESIZE;
        }
    }

    // 保存配置
    save_config(&state, &app).await.map_err(|e| format!("保存配置失败: {}", e))?;

    // 获取置顶状态
    let always_on_top = {
        let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        config.ui_config.always_on_top
    };

    // 应用到当前窗口
    if let Some(window) = app.get_webview_window("main") {
        if size_update.fixed {
            // 固定模式：设置精确的窗口大小和约束
            if let Err(e) = window.set_size(tauri::LogicalSize::new(size_update.width, size_update.height)) {
                return Err(format!("设置窗口大小失败: {}", e));
            }

            if let Err(e) = window.set_min_size(Some(tauri::LogicalSize::new(size_update.width, size_update.height))) {
                return Err(format!("设置最小窗口大小失败: {}", e));
            }

            if let Err(e) = window.set_max_size(Some(tauri::LogicalSize::new(size_update.width, size_update.height))) {
                return Err(format!("设置最大窗口大小失败: {}", e));
            }

            log::debug!("窗口已设置为固定大小: {}x{}", size_update.width, size_update.height);
        } else {
            // 自由拉伸模式：设置合理的约束范围
            if let Err(e) = window.set_min_size(Some(tauri::LogicalSize::new(window::MIN_WIDTH, window::MIN_HEIGHT))) {
                return Err(format!("设置最小窗口大小失败: {}", e));
            }

            if let Err(e) = window.set_max_size(Some(tauri::LogicalSize::new(window::MAX_WIDTH, window::MAX_HEIGHT))) {
                return Err(format!("设置最大窗口大小失败: {}", e));
            }

            // 设置为默认大小
            if let Err(e) = window.set_size(tauri::LogicalSize::new(size_update.width, size_update.height)) {
                return Err(format!("设置窗口大小失败: {}", e));
            }

            log::debug!("窗口已设置为自由拉伸模式，默认大小: {}x{}", size_update.width, size_update.height);
        }

        // 重新应用置顶状态，确保窗口大小变更不会影响置顶设置
        if let Err(e) = window.set_always_on_top(always_on_top) {
            log::warn!("重新应用置顶状态失败: {}", e);
        } else {
            log::debug!("置顶状态已重新应用: {}", always_on_top);
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn save_window_position(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let position = window.outer_position()
            .map_err(|e| format!("获取窗口位置失败: {}", e))?;

        if !validation::is_valid_window_position(position.x, position.y) {
            return Err(format!("无效的窗口位置: ({}, {})", position.x, position.y));
        }

        let px = position.x as f64;
        let py = position.y as f64;

        {
            let mut config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
            config.ui_config.window_config.position_x = Some(px);
            config.ui_config.window_config.position_y = Some(py);
        }

        save_config(&state, &app).await.map_err(|e| format!("保存配置失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn restore_window_position(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<bool, String> {
    let (pos_x, pos_y, win_config) = {
        let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        (
            config.ui_config.window_config.position_x,
            config.ui_config.window_config.position_y,
            config.ui_config.window_config.clone(),
        )
    };

    let (logical_w, logical_h) = if win_config.fixed {
        (win_config.fixed_width, win_config.fixed_height)
    } else {
        (win_config.free_width, win_config.free_height)
    };

    if let (Some(x), Some(y)) = (pos_x, pos_y) {
        log::debug!("读取到保存的窗口位置: ({}, {})", x, y);
        if let Some(win) = app.get_webview_window("main") {
            let physical_x = x as i32;
            let physical_y = y as i32;

            if center_on_monitor_containing(&win, physical_x, physical_y, logical_w, logical_h) {
                log::debug!("窗口已在上次所在显示器上居中");
                return Ok(true);
            }

            log::debug!("上次所在显示器不可用，使用主显示器居中");
            let _ = win.center();
        }
    } else {
        log::debug!("没有保存的窗口位置，使用默认位置");
    }

    Ok(false)
}
