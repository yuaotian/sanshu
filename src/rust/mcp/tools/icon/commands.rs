// Tauri 命令实现
// 提供前端可调用的图标搜索、下载、保存等功能

use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use super::api;
use super::types::{
    ClearCacheRequest, ClearCacheResult, IconCacheStats, IconConfig,
    IconContentRequest, IconContentResult, IconFormat, IconItem,
    IconSaveItem, IconSaveRequest, IconSaveResult, IconSearchRequest,
    IconSearchResult,
};
use crate::log_debug;
use crate::log_important;

// ============ 搜索命令 ============

/// 搜索图标
/// 
/// 根据关键词和筛选条件搜索 Iconfont 图标库
#[tauri::command]
pub async fn search_icons(request: IconSearchRequest) -> Result<IconSearchResult, String> {
    log_debug!("搜索图标: query={}, page={:?}", request.query, request.page);
    
    api::search_icons(request)
        .await
        .map_err(|e| {
            log_important!(error, "图标搜索失败: {}", e);
            format!("搜索失败: {}", e)
        })
}

// ============ 图标内容获取命令 ============

/// 获取图标内容
/// 
/// 获取指定图标的 SVG 或 PNG 内容
#[tauri::command]
pub async fn get_icon_content(request: IconContentRequest) -> Result<IconContentResult, String> {
    log_debug!("获取图标内容: id={}, format={:?}", request.id, request.format);
    
    // 获取 SVG 内容
    let svg_content = api::get_icon_svg(request.id, None)
        .await
        .map_err(|e| format!("获取图标内容失败: {}", e))?;
    
    match request.format {
        IconFormat::Svg | IconFormat::Both => {
            Ok(IconContentResult {
                id: request.id,
                name: format!("icon_{}", request.id),
                svg_content: Some(svg_content),
                png_base64: None, // PNG 转换暂未实现
                mime_type: "image/svg+xml".to_string(),
            })
        }
        IconFormat::Png => {
            // PNG 格式需要服务端转换，暂返回 SVG
            log_debug!("PNG 格式暂不支持，返回 SVG");
            Ok(IconContentResult {
                id: request.id,
                name: format!("icon_{}", request.id),
                svg_content: Some(svg_content),
                png_base64: None,
                mime_type: "image/svg+xml".to_string(),
            })
        }
    }
}

// ============ 保存命令 ============

/// 保存图标到本地
/// 
/// 将选中的图标保存到指定目录
#[tauri::command]
pub async fn save_icons(request: IconSaveRequest) -> Result<IconSaveResult, String> {
    log_debug!(
        "保存图标: count={}, path={}, format={:?}",
        request.icons.len(),
        request.save_path,
        request.format
    );
    
    // 确保目录存在
    let save_dir = PathBuf::from(&request.save_path);
    if !save_dir.exists() {
        fs::create_dir_all(&save_dir)
            .map_err(|e| format!("创建目录失败: {}", e))?;
    }
    
    let mut items = Vec::new();
    let mut success_count = 0;
    let mut failed_count = 0;
    
    for icon in &request.icons {
        match save_single_icon(icon, &save_dir, &request.format).await {
            Ok(saved_paths) => {
                items.push(IconSaveItem {
                    id: icon.id,
                    name: icon.name.clone(),
                    success: true,
                    saved_paths,
                    error: None,
                });
                success_count += 1;
            }
            Err(e) => {
                log_important!(error, "保存图标 {} 失败: {}", icon.id, e);
                items.push(IconSaveItem {
                    id: icon.id,
                    name: icon.name.clone(),
                    success: false,
                    saved_paths: vec![],
                    error: Some(e),
                });
                failed_count += 1;
            }
        }
    }
    
    log_debug!("图标保存完成: 成功 {}, 失败 {}", success_count, failed_count);
    
    Ok(IconSaveResult {
        items,
        success_count,
        failed_count,
        save_path: request.save_path,
    })
}

/// 保存单个图标
async fn save_single_icon(
    icon: &IconItem,
    save_dir: &PathBuf,
    format: &IconFormat,
) -> Result<Vec<String>, String> {
    let mut saved_paths = Vec::new();
    
    // 获取 SVG 内容
    let svg_content = api::get_icon_svg(icon.id, icon.svg_content.clone())
        .await
        .map_err(|e| format!("获取 SVG 失败: {}", e))?;
    
    // 生成安全的文件名
    let safe_name = sanitize_filename(&icon.name);
    
    // 保存 SVG
    if *format == IconFormat::Svg || *format == IconFormat::Both {
        let svg_path = save_dir.join(format!("{}.svg", safe_name));
        fs::write(&svg_path, &svg_content)
            .map_err(|e| format!("写入 SVG 文件失败: {}", e))?;
        saved_paths.push(svg_path.to_string_lossy().to_string());
    }
    
    // PNG 保存暂未实现（需要 SVG 转 PNG 库）
    if *format == IconFormat::Png || *format == IconFormat::Both {
        // TODO: 使用 resvg 或其他库将 SVG 转换为 PNG
        log_debug!("PNG 格式保存暂未实现");
    }
    
    Ok(saved_paths)
}

/// 清理文件名中的非法字符
fn sanitize_filename(name: &str) -> String {
    let mut safe_name = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    
    // 确保文件名不为空
    if safe_name.is_empty() {
        safe_name = "icon".to_string();
    }
    
    // 限制长度
    if safe_name.len() > 64 {
        safe_name = safe_name[..64].to_string();
    }
    
    safe_name
}

// ============ 缓存管理命令 ============

/// 获取图标缓存统计
#[tauri::command]
pub fn get_icon_cache_stats() -> IconCacheStats {
    api::get_cache_stats()
}

/// 清空图标缓存
#[tauri::command]
pub fn clear_icon_cache(request: ClearCacheRequest) -> ClearCacheResult {
    log_debug!("清空图标缓存: expired_only={}", request.expired_only);
    api::clear_cache(request.expired_only)
}

// ============ 配置管理命令 ============

/// 获取图标工坊配置
#[tauri::command]
pub fn get_icon_config() -> Result<IconConfig, String> {
    // 从配置文件加载（简化实现，直接返回默认值）
    // TODO: 集成到主配置系统
    Ok(IconConfig::default())
}

/// 保存图标工坊配置
#[tauri::command]
pub fn set_icon_config(config: IconConfig) -> Result<(), String> {
    log_debug!("更新图标工坊配置: {:?}", config);
    
    // 更新缓存过期时间
    if let Some(minutes) = config.cache_expiry_minutes {
        api::set_cache_expiry_minutes(minutes);
    }
    
    // TODO: 持久化到配置文件
    
    Ok(())
}

// ============ 剪贴板命令 ============

/// 复制 SVG 内容到剪贴板
#[tauri::command]
pub async fn copy_icon_to_clipboard(
    app_handle: tauri::AppHandle,
    icon: IconItem,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    
    log_debug!("复制图标到剪贴板: id={}", icon.id);
    
    // 获取 SVG 内容
    let svg_content = api::get_icon_svg(icon.id, icon.svg_content)
        .await
        .map_err(|e| format!("获取图标内容失败: {}", e))?;
    
    // 复制到剪贴板
    app_handle
        .clipboard()
        .write_text(&svg_content)
        .map_err(|e| format!("复制到剪贴板失败: {}", e))?;
    
    log_debug!("图标已复制到剪贴板");
    Ok(())
}

// ============ 辅助函数 ============

/// 选择保存目录
#[tauri::command]
pub async fn select_icon_save_directory(
    app_handle: tauri::AppHandle,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    
    let mut builder = app_handle.dialog().file();
    
    if let Some(path) = default_path {
        let path_buf = PathBuf::from(&path);
        if path_buf.exists() {
            builder = builder.set_directory(&path_buf);
        }
    }
    
    // 使用 tokio oneshot channel 接收回调结果
    let (tx, rx) = tokio::sync::oneshot::channel();
    
    // 选择目录（Tauri 2.0 使用回调模式）
    builder.pick_folder(move |folder_path| {
        let _ = tx.send(folder_path);
    });
    
    // 等待回调结果
    let result = rx.await.map_err(|_| "对话框选择被取消".to_string())?;
    
    Ok(result.map(|path| path.to_string()))
}
