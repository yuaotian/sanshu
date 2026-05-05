use tauri::{AppHandle, Emitter, State};
use serde::{Deserialize, Serialize};
use std::{fs, io::{Read, Write}, path::PathBuf, process::Command};
use crate::config::AppState;
use crate::network::{detect_geo_location, ProxyDetector, ProxyInfo, create_update_client, create_download_client};
use crate::network::geo::GeoLocation;

/// 网络状态信息
/// 用于向前端展示当前的网络环境和代理状态
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NetworkStatus {
    /// 当前 IP 的国家代码（如 "CN", "US"）
    pub country: String,
    /// 当前 IP 的城市（可选）
    pub city: Option<String>,
    /// 当前 IP 地址
    pub ip: Option<String>,
    /// 是否使用了代理
    pub using_proxy: bool,
    /// 代理信息（如果使用了代理）
    pub proxy_host: Option<String>,
    pub proxy_port: Option<u16>,
    pub proxy_type: Option<String>,
    /// GitHub API 是否可达
    pub github_reachable: bool,
}

impl Default for NetworkStatus {
    fn default() -> Self {
        Self {
            country: "UNKNOWN".to_string(),
            city: None,
            ip: None,
            using_proxy: false,
            proxy_host: None,
            proxy_port: None,
            proxy_type: None,
            github_reachable: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_notes: String,
    pub download_url: String,
    /// 网络状态信息（新增）
    pub network_status: NetworkStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateProgress {
    pub chunk_length: usize,
    pub content_length: Option<u64>,
    pub downloaded: u64,
    pub percentage: f64,
}

/// 检查是否有可用更新
#[tauri::command]
pub async fn check_for_updates(app: AppHandle, state: State<'_, AppState>) -> Result<UpdateInfo, String> {
    log::info!("🔍 开始检查更新");

    // 第一步：检测地理位置（用于网络状态展示）
    let geo_info = detect_geo_location_full().await;
    log::info!("🌍 地理位置检测完成: country={}, city={:?}",
        geo_info.country, geo_info.city);

    // 第二步：智能代理检测和配置
    let proxy_info = detect_and_configure_proxy(&state).await;

    // 构建网络状态信息
    let mut network_status = NetworkStatus {
        country: geo_info.country.clone(),
        city: geo_info.city.clone(),
        ip: Some(geo_info.ip.clone()),
        using_proxy: proxy_info.is_some(),
        proxy_host: proxy_info.as_ref().map(|p| p.host.clone()),
        proxy_port: proxy_info.as_ref().map(|p| p.port),
        proxy_type: proxy_info.as_ref().map(|p| p.proxy_type.to_string()),
        github_reachable: false, // 稍后更新
    };

    // 创建HTTP客户端（带或不带代理）
    let client = create_update_client(proxy_info.as_ref())
        .map_err(|e| {
            log::error!("❌ 创建HTTP客户端失败: {}", e);
            format!("创建HTTP客户端失败: {}", e)
        })?;

    log::info!("📡 发送 GitHub API 请求");

    let response = client
        .get("https://api.github.com/repos/yuaotian/sanshu/releases/latest")
        .header("User-Agent", "sanshu-app/1.0")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| {
            log::error!("❌ 网络请求失败: {}", e);
            format!("网络请求失败: {}", e)
        })?;

    log::info!("📊 GitHub API 响应状态: {}", response.status());

    // 更新 GitHub 可达状态
    network_status.github_reachable = response.status().is_success();

    if !response.status().is_success() {
        let status = response.status();
        let error_msg = if status == 403 {
            "网络请求受限，请手动下载最新版本".to_string()
        } else if status == 404 {
            "网络连接异常，请检查网络后重试".to_string()
        } else {
            format!("网络请求失败: {}", status)
        };
        log::error!("❌ {}", error_msg);
        return Err(error_msg);
    }

    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|e| {
            log::error!("❌ 解析响应失败: {}", e);
            format!("解析响应失败: {}", e)
        })?;

    log::info!("📋 成功获取 release 数据");

    let current_version = app.package_info().version.to_string();
    log::info!("📦 当前版本: {}", current_version);

    // 提取最新版本号，处理中文tag
    let tag_name = release["tag_name"]
        .as_str()
        .unwrap_or("")
        .to_string();

    log::info!("🏷️ GitHub tag: {}", tag_name);

    // 移除前缀v和中文字符，只保留数字和点
    let latest_version = tag_name
        .replace("v", "")
        .chars()
        .filter(|c| c.is_numeric() || *c == '.')
        .collect::<String>();

    log::info!("🆕 解析后的最新版本: {}", latest_version);

    if latest_version.is_empty() {
        let error_msg = "无法解析版本号".to_string();
        log::error!("❌ {}", error_msg);
        return Err(error_msg);
    }

    // 比较版本号
    let has_update = compare_versions(&latest_version, &current_version);
    log::info!("🔄 版本比较结果 - 有更新: {}", has_update);

    // 获取实际的下载URL（从assets中找到对应平台的文件）
    let download_url = get_platform_download_url(&release)?;

    let update_info = UpdateInfo {
        available: has_update,
        current_version,
        latest_version,
        release_notes: release["body"].as_str().unwrap_or("").to_string(),
        download_url,
        network_status,
    };

    log::info!("✅ 更新检查完成: {:?}", update_info);
    Ok(update_info)
}

/// 简单的版本比较函数
fn compare_versions(v1: &str, v2: &str) -> bool {
    let v1_parts: Vec<u32> = v1.split('.').filter_map(|s| s.parse().ok()).collect();
    let v2_parts: Vec<u32> = v2.split('.').filter_map(|s| s.parse().ok()).collect();
    
    let max_len = v1_parts.len().max(v2_parts.len());
    
    for i in 0..max_len {
        let v1_part = v1_parts.get(i).unwrap_or(&0);
        let v2_part = v2_parts.get(i).unwrap_or(&0);
        
        if v1_part > v2_part {
            return true;
        } else if v1_part < v2_part {
            return false;
        }
    }
    
    false
}

/// 下载并安装更新
#[tauri::command]
pub async fn download_and_install_update(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    log::info!("🚀 开始下载和安装更新");

    // 首先检查更新信息
    log::info!("🔍 重新检查更新信息");
    let update_info = check_for_updates(app.clone(), state.clone()).await?;

    log::info!("📊 更新信息: {:?}", update_info);

    if !update_info.available {
        let error_msg = "没有可用的更新".to_string();
        log::warn!("⚠️ {}", error_msg);
        return Err(error_msg);
    }

    log::info!("✅ 确认有可用更新，准备下载");

    // 发送下载开始事件
    log::info!("📢 发送下载开始事件");
    let _ = app.emit("update_download_started", ());

    // 实现真正的下载和安装逻辑
    match download_and_install_update_impl(&app, &state, &update_info).await {
        Ok(_) => {
            log::info!("✅ 更新下载和安装成功");
            let _ = app.emit("update_install_finished", ());
            
            // Windows 平台：发送自动退出事件，让前端显示倒计时并自动退出
            // 这样批处理脚本才能检测到进程退出并完成文件替换
            #[cfg(target_os = "windows")]
            {
                log::info!("🔄 Windows 平台：应用将在3秒后自动退出以完成更新");
                let _ = app.emit("update_auto_exit", 3i32); // 3秒后退出
            }
            
            Ok(())
        }
        Err(e) => {
            log::error!("❌ 更新失败: {}", e);

            // 如果自动更新失败，提供手动下载选项
            log::info!("🔗 发送手动下载事件，URL: {}", update_info.download_url);
            let _ = app.emit("update_manual_download_required", &update_info.download_url);

            // 返回更友好的错误消息
            if e.contains("手动下载") {
                Err("请手动下载最新版本".to_string())
            } else {
                Err(format!("自动更新失败，请手动下载最新版本: {}", e))
            }
        }
    }
}

/// 获取当前应用版本
#[tauri::command]
pub async fn get_current_version(app: AppHandle) -> Result<String, String> {
    Ok(app.package_info().version.to_string())
}

/// 重启应用以完成更新
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<(), String> {
    app.restart();
}

/// 更新后退出应用（专门用于 Windows 更新流程）
/// 
/// 与 restart_app 不同，此函数会完全退出进程，让批处理脚本能够检测到进程退出
/// 并执行文件替换和自动重启
#[tauri::command]
pub async fn exit_for_update() -> Result<(), String> {
    log::info!("🔄 更新完成，应用即将退出以完成文件替换...");
    // 使用延迟退出，让前端有时间显示提示
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(500));
        log::info!("👋 应用退出，批处理脚本将自动完成更新并重启");
        std::process::exit(0);
    });
    Ok(())
}

/// 获取当前平台信息
#[tauri::command]
pub fn get_platform_info() -> String {
    if cfg!(target_os = "windows") {
        "windows".to_string()
    } else if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else {
        "unknown".to_string()
    }
}

/// 获取当前平台对应的下载URL
fn get_platform_download_url(release: &serde_json::Value) -> Result<String, String> {
    let assets = release["assets"].as_array()
        .ok_or_else(|| "无法获取release assets".to_string())?;

    log::info!("📦 Release assets 总数: {}", assets.len());

    // 确定当前平台（匹配实际的文件名格式）
    let platform = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64"
        } else {
            "macos-x86_64"
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "windows-aarch64"
        } else {
            "windows-x86_64"
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "linux-aarch64"
        } else {
            "linux-x86_64"
        }
    } else {
        return Err("不支持的平台".to_string());
    };

    log::info!("🔍 查找平台 {} 的下载文件", platform);

    // 列出所有可用的 assets
    for (i, asset) in assets.iter().enumerate() {
        if let Some(name) = asset["name"].as_str() {
            log::info!("📄 Asset {}: {}", i + 1, name);
        }
    }

    // 查找对应平台的文件
    for asset in assets {
        if let Some(name) = asset["name"].as_str() {
            log::info!("🔍 检查文件: {} (是否包含 '{}')", name, platform);
            if name.contains(platform) {
                if let Some(download_url) = asset["browser_download_url"].as_str() {
                    log::info!("✅ 找到匹配的下载文件: {}", name);
                    log::info!("🔗 下载URL: {}", download_url);
                    return Ok(download_url.to_string());
                }
            }
        }
    }

    // 如果找不到对应平台的文件，返回release页面URL作为fallback
    log::warn!("⚠️ 未找到平台 {} 的下载文件，使用release页面", platform);
    log::warn!("💡 可能的原因：1. 该平台没有预编译版本 2. 文件名格式不匹配");
    Ok(release["html_url"].as_str().unwrap_or("").to_string())
}

/// 实际的下载和安装实现
async fn download_and_install_update_impl(
    app: &AppHandle,
    state: &State<'_, AppState>,
    update_info: &UpdateInfo
) -> Result<(), String> {
    log::info!("🚀 开始自动更新实现");
    log::info!("📋 更新信息: {:?}", update_info);

    // 如果下载URL是GitHub页面而不是直接下载链接，引导用户手动下载
    if update_info.download_url.contains("/releases/tag/") {
        log::info!("🔗 下载URL是release页面，需要手动下载: {}", update_info.download_url);
        log::info!("💡 这通常意味着没有找到当前平台的预编译版本");
        return Err("请手动下载最新版本".to_string());
    }

    log::info!("📥 开始下载文件: {}", update_info.download_url);

    // 创建临时目录
    let temp_dir = std::env::temp_dir().join("sanshu_update");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;

    // 确定文件名
    let file_name = update_info.download_url
        .split('/')
        .last()
        .unwrap_or("update_file")
        .to_string();

    let file_path = temp_dir.join(&file_name);

    // 智能代理检测和配置（用于下载）
    let proxy_info = detect_and_configure_proxy(state).await;

    // 创建用于下载的HTTP客户端（带或不带代理）
    let client = create_download_client(proxy_info.as_ref())
        .map_err(|e| format!("创建下载客户端失败: {}", e))?;

    let mut response = client
        .get(&update_info.download_url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("下载失败: HTTP {}", response.status()));
    }

    let total_size = response.content_length();
    let mut downloaded = 0u64;
    let mut file = fs::File::create(&file_path)
        .map_err(|e| format!("创建文件失败: {}", e))?;

    // 下载并报告进度
    while let Some(chunk) = response.chunk().await
        .map_err(|e| format!("下载数据失败: {}", e))? {

        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败: {}", e))?;

        downloaded += chunk.len() as u64;

        let percentage = if let Some(total) = total_size {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let progress = UpdateProgress {
            chunk_length: chunk.len(),
            content_length: total_size,
            downloaded,
            percentage,
        };

        let _ = app.emit("update_download_progress", &progress);
    }

    log::info!("✅ 文件下载完成: {}", file_path.display());

    // 开始安装
    let _ = app.emit("update_install_started", ());

    // 根据平台执行不同的安装逻辑
    install_update(&file_path).await?;

    Ok(())
}

/// 根据平台安装更新
async fn install_update(file_path: &PathBuf) -> Result<(), String> {
    log::info!("🔧 开始安装更新: {}", file_path.display());

    if cfg!(target_os = "macos") {
        install_macos_update(file_path).await
    } else if cfg!(target_os = "windows") {
        install_windows_update(file_path).await
    } else if cfg!(target_os = "linux") {
        install_linux_update(file_path).await
    } else {
        Err("不支持的平台".to_string())
    }
}

/// macOS 安装逻辑
async fn install_macos_update(file_path: &PathBuf) -> Result<(), String> {
    let file_name = file_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if file_name.ends_with(".tar.gz") {
        // 压缩包文件，需要解压并替换当前可执行文件
        log::info!("📦 处理 tar.gz 压缩包文件");
        install_from_archive(file_path).await
    } else if file_name.ends_with(".dmg") {
        // DMG 文件需要挂载后复制
        log::info!("📦 处理 DMG 文件");
        return Err("DMG 文件需要手动安装，请手动下载最新版本".to_string());
    } else {
        return Err("未知的文件格式，请手动下载最新版本".to_string());
    }
}

/// Windows 安装逻辑
async fn install_windows_update(file_path: &PathBuf) -> Result<(), String> {
    let file_name = file_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if file_name.ends_with(".zip") {
        // ZIP 压缩包文件，需要解压并替换当前可执行文件
        log::info!("📦 处理 ZIP 压缩包文件");
        install_from_archive(file_path).await
    } else if file_name.ends_with(".msi") {
        // MSI 安装包
        log::info!("📦 执行 MSI 安装");
        let output = Command::new("msiexec")
            .args(&["/i", file_path.to_str().unwrap(), "/quiet"])
            .output()
            .map_err(|e| format!("执行 MSI 安装失败: {}", e))?;

        if !output.status.success() {
            return Err(format!("MSI 安装失败: {}", String::from_utf8_lossy(&output.stderr)));
        }

        Ok(())
    } else if file_name.ends_with(".exe") {
        // EXE 安装包
        log::info!("📦 执行 EXE 安装");
        let output = Command::new(file_path)
            .args(&["/S"]) // 静默安装
            .output()
            .map_err(|e| format!("执行 EXE 安装失败: {}", e))?;

        if !output.status.success() {
            return Err(format!("EXE 安装失败: {}", String::from_utf8_lossy(&output.stderr)));
        }

        Ok(())
    } else {
        Err("未知的文件格式，请手动下载最新版本".to_string())
    }
}

/// Linux 安装逻辑
async fn install_linux_update(file_path: &PathBuf) -> Result<(), String> {
    let file_name = file_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if file_name.ends_with(".tar.gz") {
        // 压缩包文件，需要解压并替换当前可执行文件
        log::info!("📦 处理 tar.gz 压缩包文件");
        install_from_archive(file_path).await
    } else if file_name.ends_with(".deb") {
        // DEB 包
        log::info!("📦 执行 DEB 安装");
        let output = Command::new("dpkg")
            .args(&["-i", file_path.to_str().unwrap()])
            .output()
            .map_err(|e| format!("执行 DEB 安装失败: {}", e))?;

        if !output.status.success() {
            return Err(format!("DEB 安装失败: {}", String::from_utf8_lossy(&output.stderr)));
        }

        Ok(())
    } else if file_name.ends_with(".rpm") {
        // RPM 包
        log::info!("📦 执行 RPM 安装");
        let output = Command::new("rpm")
            .args(&["-U", file_path.to_str().unwrap()])
            .output()
            .map_err(|e| format!("执行 RPM 安装失败: {}", e))?;

        if !output.status.success() {
            return Err(format!("RPM 安装失败: {}", String::from_utf8_lossy(&output.stderr)));
        }

        Ok(())
    } else {
        Err("未知的文件格式，请手动下载最新版本".to_string())
    }
}

/// 从压缩包安装更新（支持多文件更新）
async fn install_from_archive(file_path: &PathBuf) -> Result<(), String> {
    log::info!("📦 开始从压缩包安装更新: {}", file_path.display());

    // 获取当前可执行文件的路径和所在目录
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("无法获取当前可执行文件路径: {}", e))?;

    let app_dir = current_exe.parent()
        .ok_or_else(|| "无法获取应用程序目录".to_string())?
        .to_path_buf();

    log::info!("📍 当前可执行文件路径: {}", current_exe.display());
    log::info!("📂 应用程序目录: {}", app_dir.display());

    // 创建临时解压目录
    let temp_dir = std::env::temp_dir().join("sanshu_extract");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)
            .map_err(|e| format!("清理临时目录失败: {}", e))?;
    }
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时解压目录失败: {}", e))?;

    log::info!("📂 临时解压目录: {}", temp_dir.display());

    // 根据文件类型解压，获取解压后的文件列表
    let file_name = file_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let extracted_files = if file_name.ends_with(".tar.gz") {
        extract_tar_gz(file_path, &temp_dir)?
    } else if file_name.ends_with(".zip") {
        extract_zip(file_path, &temp_dir)?
    } else {
        return Err("不支持的压缩格式".to_string());
    };

    log::info!("📋 解压完成，共 {} 个文件需要更新", extracted_files.len());
    for file in &extracted_files {
        log::info!("  📄 {}", file.display());
    }

    // 根据平台执行不同的替换策略
    if cfg!(target_os = "windows") {
        // Windows: 使用批处理脚本延迟替换所有文件
        replace_all_files_windows(&app_dir, &temp_dir, &extracted_files)?;
    } else {
        // macOS/Linux: 直接替换所有文件
        replace_all_files_unix(&app_dir, &extracted_files)?;
        // 清理临时目录（Unix 平台可以立即清理）
        let _ = fs::remove_dir_all(&temp_dir);
    }

    log::info!("✅ 更新安装完成！");
    Ok(())
}

/// 解压 tar.gz 文件
fn extract_tar_gz(archive_path: &PathBuf, extract_to: &PathBuf) -> Result<Vec<PathBuf>, String> {
    log::info!("📦 解压 tar.gz 文件: {}", archive_path.display());

    let output = Command::new("tar")
        .args(&["-xzf", archive_path.to_str().unwrap(), "-C", extract_to.to_str().unwrap()])
        .output()
        .map_err(|e| format!("执行 tar 命令失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("tar 解压失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    log::info!("✅ tar.gz 解压完成");

    // 收集解压后的所有文件
    let files = collect_files_in_dir(extract_to)?;
    log::info!("📋 tar.gz 解压后找到 {} 个文件", files.len());

    if files.is_empty() {
        return Err("tar.gz 解压完成但没有提取到任何文件".to_string());
    }

    Ok(files)
}

/// 递归收集目录中的所有文件
fn collect_files_in_dir(dir: &PathBuf) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();

    if !dir.exists() {
        log::error!("❌ 目录不存在: {}", dir.display());
        return Err(format!("目录不存在: {}", dir.display()));
    }

    fn collect_recursive(dir: &PathBuf, files: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("读取目录失败 {}: {}", dir.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
            let path = entry.path();

            if path.is_dir() {
                collect_recursive(&path, files)?;
            } else {
                log::info!("📄 发现文件: {}", path.display());
                files.push(path);
            }
        }
        Ok(())
    }

    collect_recursive(dir, &mut files)?;
    Ok(files)
}

/// 解压 zip 文件（使用 Rust 原生 zip crate，正确处理中文文件名）
fn extract_zip(archive_path: &PathBuf, extract_to: &PathBuf) -> Result<Vec<PathBuf>, String> {
    log::info!("📦 开始解压 zip 文件: {}", archive_path.display());
    log::info!("📂 解压目标目录: {}", extract_to.display());

    // 打开 ZIP 文件
    let file = fs::File::open(archive_path)
        .map_err(|e| format!("无法打开 ZIP 文件: {}", e))?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("无法读取 ZIP 归档: {}", e))?;

    log::info!("📋 ZIP 文件包含 {} 个条目", archive.len());

    let mut extracted_files: Vec<PathBuf> = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("无法读取 ZIP 条目 {}: {}", i, e))?;

        // 获取文件名（正确处理 UTF-8 编码的中文文件名）
        let file_name = file.name().to_string();
        log::info!("📄 处理条目 {}: {}", i + 1, file_name);

        // 构建目标路径
        let out_path = extract_to.join(&file_name);

        if file.is_dir() {
            // 创建目录
            log::info!("📁 创建目录: {}", out_path.display());
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("创建目录失败 {}: {}", out_path.display(), e))?;
        } else {
            // 确保父目录存在
            if let Some(parent) = out_path.parent() {
                if !parent.exists() {
                    log::info!("📁 创建父目录: {}", parent.display());
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("创建父目录失败 {}: {}", parent.display(), e))?;
                }
            }

            // 解压文件
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败 {}: {}", out_path.display(), e))?;

            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .map_err(|e| format!("读取 ZIP 条目内容失败: {}", e))?;

            out_file.write_all(&buffer)
                .map_err(|e| format!("写入文件失败 {}: {}", out_path.display(), e))?;

            let file_size = buffer.len();
            log::info!("✅ 解压文件: {} ({} 字节)", out_path.display(), file_size);

            extracted_files.push(out_path);
        }
    }

    log::info!("✅ ZIP 解压完成，共解压 {} 个文件", extracted_files.len());

    // 验证解压结果
    if extracted_files.is_empty() {
        return Err("ZIP 解压完成但没有提取到任何文件".to_string());
    }

    Ok(extracted_files)
}

/// 返回 MCP 中文入口对应的 ASCII 兼容别名。
///
/// 中文说明：部分 MCP 客户端或终端对中文命令解析不稳定，因此更新时需要同步生成 `sanshu` 入口。
fn mcp_ascii_alias(file_name: &str) -> Option<&'static str> {
    match file_name {
        "三术.exe" => Some("sanshu.exe"),
        "三术" => Some("sanshu"),
        _ => None,
    }
}

/// Windows 平台替换所有文件（使用批处理脚本延迟替换）
///
/// # 参数
/// - `app_dir`: 应用程序目录（目标目录）
/// - `extract_dir`: 解压临时目录（源目录）
/// - `files`: 需要替换的文件列表（在 extract_dir 中的路径）
fn replace_all_files_windows(
    app_dir: &PathBuf,
    extract_dir: &PathBuf,
    files: &[PathBuf]
) -> Result<(), String> {
    log::info!("🔧 Windows 平台：准备批处理脚本替换 {} 个文件", files.len());

    // 获取当前可执行文件名（用于重启）
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("无法获取当前可执行文件路径: {}", e))?;
    let exe_name = current_exe.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("等一下.exe");

    let script_path = app_dir.join("update_script.bat");

    // 构建批处理脚本内容
    let mut script_lines = Vec::new();

    // 脚本头部：设置编码和关闭回显
    script_lines.push("@echo off".to_string());
    script_lines.push("chcp 65001 >nul".to_string());
    script_lines.push("setlocal enabledelayedexpansion".to_string());
    script_lines.push("echo 正在更新 sanshu...".to_string());
    // 等待应用进程退出，避免可执行文件被占用导致更新失败
    script_lines.push("timeout /t 5 /nobreak >nul".to_string());
    script_lines.push(format!("set \"APP_EXE={}\"", exe_name));
    script_lines.push("set /a WAIT_MAX=30".to_string());
    script_lines.push("set /a WAIT_SEC=0".to_string());
    script_lines.push("echo 等待应用进程退出：%APP_EXE%".to_string());
    script_lines.push(":wait_app_exit".to_string());
    script_lines.push("tasklist /FI \"IMAGENAME eq %APP_EXE%\" | find /I \"%APP_EXE%\" >nul".to_string());
    script_lines.push("if errorlevel 1 goto wait_done".to_string());
    script_lines.push("if !WAIT_SEC! GEQ !WAIT_MAX! goto wait_timeout".to_string());
    script_lines.push("timeout /t 1 /nobreak >nul".to_string());
    script_lines.push("set /a WAIT_SEC+=1".to_string());
    script_lines.push("goto wait_app_exit".to_string());
    script_lines.push(":wait_timeout".to_string());
    script_lines.push("echo 等待超时，继续尝试更新...".to_string());
    script_lines.push(":wait_done".to_string());
    script_lines.push("".to_string());

    // 备份和复制每个文件
    for file in files {
        let file_name = file.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("无法获取文件名: {}", file.display()))?;

        let source_path = file.display().to_string();
        let target_path = app_dir.join(file_name);
        let target_path_str = target_path.display().to_string();
        let backup_path = app_dir.join(format!("{}.bak", file_name));
        let backup_path_str = backup_path.display().to_string();

        // 备份旧文件（如果存在）
        script_lines.push(format!("if exist \"{}\" (", target_path_str));
        script_lines.push(format!("    copy /y \"{}\" \"{}\" >nul", target_path_str, backup_path_str));
        script_lines.push(")".to_string());

        // 复制新文件
        script_lines.push(format!("copy /y \"{}\" \"{}\"", source_path, target_path_str));
        script_lines.push(format!("if errorlevel 1 ("));
        script_lines.push(format!("    echo 复制 {} 失败", file_name));
        script_lines.push(format!(") else ("));
        script_lines.push(format!("    echo 已更新: {}", file_name));
        script_lines.push(format!(")"));
        script_lines.push("".to_string());

        log::info!("📝 添加文件替换命令: {} -> {}", source_path, target_path_str);

        if let Some(alias_file_name) = mcp_ascii_alias(file_name) {
            let alias_target_path = app_dir.join(alias_file_name);
            let alias_target_path_str = alias_target_path.display().to_string();
            let alias_backup_path = app_dir.join(format!("{}.bak", alias_file_name));
            let alias_backup_path_str = alias_backup_path.display().to_string();

            // 中文说明：`sanshu.exe` 与 `三术.exe` 内容一致，用于兼容不稳定支持中文命令的 MCP 客户端。
            script_lines.push(format!("if exist \"{}\" (", alias_target_path_str));
            script_lines.push(format!(
                "    copy /y \"{}\" \"{}\" >nul",
                alias_target_path_str,
                alias_backup_path_str
            ));
            script_lines.push(")".to_string());
            script_lines.push(format!("copy /y \"{}\" \"{}\"", source_path, alias_target_path_str));
            script_lines.push(format!("if errorlevel 1 ("));
            script_lines.push(format!("    echo 复制 {} 失败", alias_file_name));
            script_lines.push(format!(") else ("));
            script_lines.push(format!("    echo 已更新: {}", alias_file_name));
            script_lines.push(format!(")"));
            script_lines.push("".to_string());

            log::info!(
                "📝 添加 MCP ASCII 别名替换命令: {} -> {}",
                source_path,
                alias_target_path_str
            );
        }
    }

    // 清理临时目录
    script_lines.push("echo 清理临时文件...".to_string());
    script_lines.push(format!("rmdir /s /q \"{}\" 2>nul", extract_dir.display()));
    script_lines.push("".to_string());

    // 重启应用
    script_lines.push("echo 重启应用...".to_string());
    let restart_exe_path = app_dir.join(exe_name);
    script_lines.push(format!("start \"\" \"{}\"", restart_exe_path.display()));
    script_lines.push("".to_string());

    // 删除脚本自身
    script_lines.push("del \"%~f0\"".to_string());

    let script_content = script_lines.join("\r\n");

    // 写入脚本文件（使用 UTF-8 with BOM 以支持中文）
    let mut file = fs::File::create(&script_path)
        .map_err(|e| format!("创建更新脚本失败: {}", e))?;

    // 写入 UTF-8 BOM
    file.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| format!("写入 BOM 失败: {}", e))?;

    file.write_all(script_content.as_bytes())
        .map_err(|e| format!("写入脚本内容失败: {}", e))?;

    log::info!("📝 创建 Windows 更新脚本: {}", script_path.display());
    log::info!("⚠️ Windows 平台需要重启应用以完成更新");

    // 启动脚本（在独立进程中运行，不等待）
    Command::new("cmd")
        .args(&["/C", "start", "/min", "", script_path.to_str().unwrap()])
        .spawn()
        .map_err(|e| format!("启动更新脚本失败: {}", e))?;

    log::info!("🚀 更新脚本已启动，应用将在退出后自动更新并重启");

    Ok(())
}

/// Unix 平台替换所有文件（直接替换）
///
/// # 参数
/// - `app_dir`: 应用程序目录（目标目录）
/// - `files`: 需要替换的文件列表（源文件路径）
fn replace_all_files_unix(app_dir: &PathBuf, files: &[PathBuf]) -> Result<(), String> {
    log::info!("🔧 Unix 平台：直接替换 {} 个文件", files.len());

    for file in files {
        let file_name = file.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("无法获取文件名: {}", file.display()))?;

        let target_path = app_dir.join(file_name);

        // 备份旧文件（如果存在）
        if target_path.exists() {
            let backup_path = app_dir.join(format!("{}.bak", file_name));
            fs::copy(&target_path, &backup_path)
                .map_err(|e| format!("备份文件失败 {}: {}", file_name, e))?;
            log::info!("💾 已备份: {} -> {}", target_path.display(), backup_path.display());
        }

        // 复制新文件
        fs::copy(file, &target_path)
            .map_err(|e| format!("复制文件失败 {}: {}", file_name, e))?;

        // 设置执行权限（对于可执行文件）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file_name.ends_with(".exe") || !file_name.contains('.') {
                let mut perms = fs::metadata(&target_path)
                    .map_err(|e| format!("获取文件权限失败: {}", e))?
                    .permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target_path, perms)
                    .map_err(|e| format!("设置执行权限失败: {}", e))?;
                log::info!("🔐 已设置执行权限: {}", target_path.display());
            }
        }

        log::info!("✅ 已更新: {}", file_name);

        if let Some(alias_file_name) = mcp_ascii_alias(file_name) {
            let alias_target_path = app_dir.join(alias_file_name);

            // 中文说明：`sanshu` 与 `三术` 内容一致，优先给 MCP 客户端使用 ASCII 命令名。
            if alias_target_path.exists() {
                let alias_backup_path = app_dir.join(format!("{}.bak", alias_file_name));
                fs::copy(&alias_target_path, &alias_backup_path)
                    .map_err(|e| format!("备份别名文件失败 {}: {}", alias_file_name, e))?;
                log::info!(
                    "💾 已备份别名: {} -> {}",
                    alias_target_path.display(),
                    alias_backup_path.display()
                );
            }

            fs::copy(file, &alias_target_path)
                .map_err(|e| format!("复制别名文件失败 {}: {}", alias_file_name, e))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&alias_target_path)
                    .map_err(|e| format!("获取别名文件权限失败: {}", e))?
                    .permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&alias_target_path, perms)
                    .map_err(|e| format!("设置别名执行权限失败: {}", e))?;
                log::info!("🔐 已设置别名执行权限: {}", alias_target_path.display());
            }

            log::info!("✅ 已更新 MCP ASCII 别名: {}", alias_file_name);
        }
    }

    log::info!("✅ Unix 平台所有文件替换完成");
    log::info!("⚠️ 建议重启应用以加载新版本");

    Ok(())
}

/// 智能代理检测和配置
///
/// 根据配置和地理位置，自动检测并配置代理
///
/// # 工作流程
/// 1. 读取代理配置
/// 2. 如果启用自动检测：
///    - 检测IP地理位置
///    - 如果在中国大陆且配置了仅CN使用代理，则检测本地代理
///    - 否则使用直连
/// 3. 如果启用手动代理：
///    - 直接使用配置的代理
/// 4. 否则使用直连
///
/// # 返回值
/// - `Some(ProxyInfo)`: 使用代理
/// - `None`: 使用直连
async fn detect_and_configure_proxy(state: &State<'_, AppState>) -> Option<ProxyInfo> {
    // 读取代理配置
    let proxy_config = {
        let config = state.config.lock().ok()?;
        config.proxy_config.clone()
    };

    log::info!("📋 代理配置: auto_detect={}, enabled={}, only_for_cn={}",
        proxy_config.auto_detect, proxy_config.enabled, proxy_config.only_for_cn);

    // 如果启用自动检测
    if proxy_config.auto_detect {
        log::info!("🔍 启用自动代理检测");

        // 检测地理位置
        let country = detect_geo_location().await;
        log::info!("🌍 检测到国家代码: {}", country);

        // 判断是否需要使用代理
        let should_use_proxy = if proxy_config.only_for_cn {
            // 仅在中国大陆使用代理
            country == "CN"
        } else {
            // 所有地区都尝试使用代理
            true
        };

        if should_use_proxy {
            log::info!("✅ 满足代理使用条件，开始检测本地代理");

            // 检测本地可用代理
            if let Some(proxy_info) = ProxyDetector::detect_available_proxy().await {
                log::info!("✅ 使用自动检测的代理: {}:{} ({})",
                    proxy_info.host, proxy_info.port, proxy_info.proxy_type);
                return Some(proxy_info);
            } else {
                log::warn!("⚠️ 未检测到可用代理，使用直连");
                return None;
            }
        } else {
            log::info!("ℹ️ 不满足代理使用条件（非CN地区），使用直连");
            return None;
        }
    }

    // 如果启用手动代理
    if proxy_config.enabled {
        log::info!("🔧 使用手动配置的代理");

        let proxy_type = match proxy_config.proxy_type.as_str() {
            "socks5" => crate::network::proxy::ProxyType::Socks5,
            _ => crate::network::proxy::ProxyType::Http,
        };

        let proxy_info = ProxyInfo::new(
            proxy_type,
            proxy_config.host,
            proxy_config.port,
        );

        log::info!("✅ 使用手动代理: {}:{} ({})",
            proxy_info.host, proxy_info.port, proxy_info.proxy_type);

        return Some(proxy_info);
    }

    log::info!("ℹ️ 未启用代理，使用直连");
    None
}

/// 检测完整的地理位置信息
///
/// 与 `detect_geo_location` 不同，此函数返回完整的 GeoLocation 结构体
/// 包含 IP、城市、国家等详细信息
async fn detect_geo_location_full() -> GeoLocation {
    log::info!("🌍 开始检测完整地理位置信息");

    // 创建HTTP客户端，设置较短的超时时间
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("⚠️ 创建HTTP客户端失败: {}", e);
            return GeoLocation {
                ip: "unknown".to_string(),
                city: None,
                region: None,
                country: "UNKNOWN".to_string(),
                loc: None,
                org: None,
                postal: None,
                timezone: None,
            };
        }
    };

    // 请求 ipinfo.io API
    match client
        .get("https://ipinfo.io/json")
        .send()
        .await
    {
        Ok(response) => {
            if !response.status().is_success() {
                log::warn!("⚠️ IP地理位置检测请求失败: HTTP {}", response.status());
                return GeoLocation {
                    ip: "unknown".to_string(),
                    city: None,
                    region: None,
                    country: "UNKNOWN".to_string(),
                    loc: None,
                    org: None,
                    postal: None,
                    timezone: None,
                };
            }

            // 解析JSON响应
            match response.json::<GeoLocation>().await {
                Ok(geo) => {
                    log::info!("✅ 检测到地理位置: {} ({}) - IP: {}",
                        geo.country,
                        geo.city.as_deref().unwrap_or("未知城市"),
                        geo.ip);
                    geo
                }
                Err(e) => {
                    log::warn!("⚠️ 解析地理位置信息失败: {}", e);
                    GeoLocation {
                        ip: "unknown".to_string(),
                        city: None,
                        region: None,
                        country: "UNKNOWN".to_string(),
                        loc: None,
                        org: None,
                        postal: None,
                        timezone: None,
                    }
                }
            }
        }
        Err(e) => {
            log::warn!("⚠️ IP地理位置检测网络请求失败: {}", e);
            GeoLocation {
                ip: "unknown".to_string(),
                city: None,
                region: None,
                country: "UNKNOWN".to_string(),
                loc: None,
                org: None,
                postal: None,
                timezone: None,
            }
        }
    }
}
