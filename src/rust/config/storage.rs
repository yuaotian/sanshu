use anyhow::Result;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, LogicalSize, Manager, State};

use super::settings::{AppConfig, AppState, default_shortcuts, default_custom_prompts};

/// 原子写入：先写临时文件再 rename，防止进程崩溃导致文件损坏
fn atomic_write_file(path: &Path, data: &[u8]) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");

    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// 安全读取配置文件：文件不存在/被锁/为空/超时均降级为 None
fn safe_read_config(path: &Path) -> Option<String> {
    use std::sync::mpsc;
    use std::time::Duration;

    if !path.exists() {
        return None;
    }

    let path = path.to_path_buf();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let _ = tx.send(fs::read_to_string(&path));
    });

    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(content)) if !content.trim().is_empty() => Some(content),
        Ok(Ok(_)) => {
            log::warn!("配置文件为空，将使用默认配置");
            None
        }
        Ok(Err(e)) => {
            log::warn!("读取配置文件失败: {}，将使用默认配置", e);
            None
        }
        Err(_) => {
            log::warn!("读取配置文件超时（可能被其他进程锁定），将使用默认配置");
            None
        }
    }
}

pub fn get_config_path(_app: &AppHandle) -> Result<PathBuf> {
    // 使用与独立配置相同的路径，确保一致性
    get_standalone_config_path()
}

pub async fn save_config(state: &State<'_, AppState>, app: &AppHandle) -> Result<()> {
    let config_path = get_config_path(app)?;

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let config = state
        .config
        .lock()
        .map_err(|e| anyhow::anyhow!("获取配置失败: {}", e))?;
    let config_json = serde_json::to_string_pretty(&*config)?;

    atomic_write_file(&config_path, config_json.as_bytes())?;

    log::debug!("配置已保存到: {:?}", config_path);

    Ok(())
}

/// Tauri应用专用的配置加载函数
pub async fn load_config(state: &State<'_, AppState>, app: &AppHandle) -> Result<()> {
    let config_path = get_config_path(app)?;

    let mut config = if let Some(json) = safe_read_config(&config_path) {
        serde_json::from_str::<AppConfig>(&json).unwrap_or_else(|e| {
            log::warn!("配置文件解析失败，使用默认配置: {}", e);
            AppConfig::default()
        })
    } else {
        AppConfig::default()
    };

    merge_default_shortcuts(&mut config);
    load_custom_prompts_into(&mut config);
    merge_default_custom_prompts(&mut config);

    let mut config_guard = state
        .config
        .lock()
        .map_err(|e| anyhow::anyhow!("获取配置锁失败: {}", e))?;
    *config_guard = config;

    Ok(())
}

pub async fn load_config_and_apply_window_settings(
    state: &State<'_, AppState>,
    app: &AppHandle,
) -> Result<()> {
    // 先加载配置（失败不阻塞窗口显示）
    if let Err(e) = load_config(state, app).await {
        log::warn!("加载配置失败，使用默认配置: {}", e);
    }

    // 然后应用窗口设置
    let (always_on_top, window_config) = {
        let config = state
            .config
            .lock()
            .map_err(|e| anyhow::anyhow!("获取配置失败: {}", e))?;
        (
            config.ui_config.always_on_top,
            config.ui_config.window_config.clone(),
        )
    };

    // 应用到窗口
    if let Some(window) = app.get_webview_window("main") {
        // 应用置顶设置
        if let Err(e) = window.set_always_on_top(always_on_top) {
            log::warn!("设置窗口置顶失败: {}", e);
        } else {
            log::info!("窗口置顶状态已设置为: {} (配置加载时)", always_on_top);
        }

        // 应用窗口大小约束
        if let Err(e) = window.set_min_size(Some(LogicalSize::new(
            window_config.min_width,
            window_config.min_height,
        ))) {
            log::warn!("设置最小窗口大小失败: {}", e);
        }

        if let Err(e) = window.set_max_size(Some(LogicalSize::new(
            window_config.max_width,
            window_config.max_height,
        ))) {
            log::warn!("设置最大窗口大小失败: {}", e);
        }

        // 根据当前模式设置窗口大小
        let (target_width, target_height) = if window_config.fixed {
            // 固定模式：使用固定尺寸
            (window_config.fixed_width, window_config.fixed_height)
        } else {
            // 自由拉伸模式：使用自由拉伸尺寸
            (window_config.free_width, window_config.free_height)
        };

        // 应用窗口大小（移除调试信息）
        if let Err(_e) = window.set_size(LogicalSize::new(target_width, target_height)) {
            // 静默处理窗口大小设置失败
        }

        // 恢复到上次所在显示器（多显示器支持）
        let positioned = if let (Some(x), Some(y)) = (window_config.position_x, window_config.position_y) {
            let physical_x = x as i32;
            let physical_y = y as i32;
            log::info!("📍 恢复窗口位置: 物理坐标 ({}, {})", physical_x, physical_y);

            if crate::ui::window::center_on_monitor_containing(
                &window, physical_x, physical_y, target_width, target_height,
            ) {
                true
            } else {
                log::info!("⚠️ 上次所在显示器不可用，使用主显示器居中");
                let _ = window.center();
                false
            }
        } else {
            let _ = window.center();
            false
        };

        // window.show() 由前端在渲染完成后调用，避免黑屏闪烁
        if positioned {
            log::info!("✅ 窗口已在上次所在显示器上居中显示");
        }
    } else {
        log::error!("找不到 main 窗口，无法应用设置");
    }

    Ok(())
}

/// 独立加载配置文件（用于MCP服务器等独立进程）
pub fn load_standalone_config() -> Result<AppConfig> {
    let config_path = get_standalone_config_path()?;

    let mut config = if let Some(json) = safe_read_config(&config_path) {
        serde_json::from_str::<AppConfig>(&json).unwrap_or_else(|e| {
            log::warn!("配置文件解析失败（{}），使用默认配置", e);
            AppConfig::default()
        })
    } else {
        AppConfig::default()
    };

    merge_default_shortcuts(&mut config);
    load_custom_prompts_into(&mut config);
    merge_default_custom_prompts(&mut config);

    Ok(config)
}

/// 独立加载Telegram配置（用于MCP模式下的配置检查）
pub fn load_standalone_telegram_config() -> Result<super::settings::TelegramConfig> {
    let config = load_standalone_config()?;
    Ok(config.telegram_config)
}

/// 获取独立配置文件路径（不依赖Tauri）
fn get_standalone_config_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join("config.json"))
}

fn get_config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("无法获取配置目录"))
        .map(|d| d.join("sanshu"))
}

fn get_custom_prompts_path() -> Result<PathBuf> {
    Ok(get_config_dir()?.join("custom_prompts.json"))
}

/// 保存自定义提示词到独立文件
pub async fn save_custom_prompts(state: &State<'_, AppState>, _app: &AppHandle) -> Result<()> {
    let prompts_path = get_custom_prompts_path()?;

    if let Some(parent) = prompts_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let prompt_config = {
        let config = state.config.lock()
            .map_err(|e| anyhow::anyhow!("获取配置失败: {}", e))?;
        config.custom_prompt_config.clone()
    };

    let json = serde_json::to_string_pretty(&prompt_config)?;
    atomic_write_file(&prompts_path, json.as_bytes())?;

    log::debug!("自定义提示词已保存到: {:?}", prompts_path);
    Ok(())
}

/// 加载自定义提示词（优先从独立文件，不存在则从 config.json 迁移）
pub fn load_custom_prompts_into(config: &mut AppConfig) {
    let prompts_path = match get_custom_prompts_path() {
        Ok(p) => p,
        Err(_) => return,
    };

    if let Some(json) = safe_read_config(&prompts_path) {
        match serde_json::from_str::<super::settings::CustomPromptConfig>(&json) {
            Ok(prompt_config) => {
                config.custom_prompt_config = prompt_config;
            }
            Err(e) => {
                log::warn!("自定义提示词文件解析失败: {}，保留主配置中的数据", e);
            }
        }
    } else if !config.custom_prompt_config.prompts.is_empty() {
        // custom_prompts.json 不存在但 config.json 有数据 → 迁移
        if let Some(parent) = prompts_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&config.custom_prompt_config) {
            if atomic_write_file(&prompts_path, json.as_bytes()).is_ok() {
                log::info!("已从 config.json 迁移自定义提示词到独立文件");
            }
        }
    }
}

/// 合并默认快捷键配置，确保新的默认快捷键被添加到现有配置中
fn merge_default_shortcuts(config: &mut AppConfig) {
    let default_shortcuts = default_shortcuts();

    // 遍历所有默认快捷键
    for (key, default_binding) in default_shortcuts {
        if !config.shortcut_config.shortcuts.contains_key(&key) {
            // 如果用户配置中不存在，则添加
            config.shortcut_config.shortcuts.insert(key, default_binding);
        } else if key == "enhance" {
            // 特殊处理：更新增强快捷键的默认值从 Shift+Enter 到 Ctrl+Shift+Enter
            let existing_binding = config.shortcut_config.shortcuts.get(&key).unwrap();

            // 检查是否是旧的默认值 (Shift+Enter)
            if existing_binding.key_combination.key == "Enter"
                && !existing_binding.key_combination.ctrl
                && existing_binding.key_combination.shift
                && !existing_binding.key_combination.alt
                && !existing_binding.key_combination.meta {
                // 更新为新的默认值 (Ctrl+Shift+Enter)
                config.shortcut_config.shortcuts.insert(key, default_binding);
            }
        }
    }
}

/// 合并默认自定义提示词配置，确保新的默认提示词被添加到现有配置中
/// 保留用户对已有提示词的修改（如 current_state、template_true 等）
fn merge_default_custom_prompts(config: &mut AppConfig) {
    let default_prompts = default_custom_prompts();

    // 遍历所有默认提示词
    for default_prompt in default_prompts {
        // 检查用户配置中是否已存在该提示词（按 ID 匹配）
        let exists = config.custom_prompt_config.prompts
            .iter()
            .any(|p| p.id == default_prompt.id);

        if !exists {
            // 用户配置中不存在，添加新的默认提示词
            config.custom_prompt_config.prompts.push(default_prompt);
        }
        // 如果存在，保留用户的修改，不覆盖
    }

    // 按 sort_order 重新排序，确保显示顺序正确
    config.custom_prompt_config.prompts
        .sort_by(|a, b| a.sort_order.cmp(&b.sort_order));
}
