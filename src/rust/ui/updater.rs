use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use crate::config::AppState;
use std::path::{Path, PathBuf};

const GITHUB_API_URL: &str = "https://api.github.com/repos/Yueby/sanshu/releases/latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_notes: String,
    pub release_url: String,
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percentage: f64,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

fn build_client(state: &AppState) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("sanshu-updater");

    if let Ok(config) = state.config.lock() {
        if config.proxy_config.enabled {
            let proxy_url = format!(
                "{}://{}:{}",
                config.proxy_config.proxy_type,
                config.proxy_config.host,
                config.proxy_config.port
            );
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build().map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

fn pick_asset_url(assets: &[GithubAsset]) -> Option<String> {
    let target = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        return None;
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };

    let pattern = format!("{}-{}", target, arch);
    assets.iter()
        .find(|a| a.name.contains(&pattern) && (a.name.ends_with(".zip") || a.name.ends_with(".tar.gz")))
        .map(|a| a.browser_download_url.clone())
}

fn compare_versions(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.trim_start_matches('v').split('.').filter_map(|s| s.parse().ok()).collect()
    };
    let c = parse(current);
    let l = parse(latest);
    l > c
}

#[tauri::command]
pub async fn check_for_updates(state: State<'_, AppState>) -> Result<UpdateInfo, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let client = build_client(&state)?;

    let release: GithubRelease = client
        .get(GITHUB_API_URL)
        .send()
        .await
        .map_err(|e| format!("请求 GitHub API 失败: {}", e))?
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let available = compare_versions(&current_version, &latest_version);
    let download_url = pick_asset_url(&release.assets).unwrap_or_default();

    Ok(UpdateInfo {
        available,
        current_version,
        latest_version,
        release_notes: release.body.unwrap_or_default(),
        release_url: release.html_url,
        download_url,
    })
}

#[tauri::command]
pub async fn download_and_apply_update(app: AppHandle, state: State<'_, AppState>, download_url: String) -> Result<String, String> {
    let client = build_client(&state)?;

    let response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {}", e))?;

    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut bytes = Vec::new();

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载中断: {}", e))?;
        downloaded += chunk.len() as u64;
        bytes.extend_from_slice(&chunk);

        let percentage = if total > 0 { (downloaded as f64 / total as f64) * 100.0 } else { 0.0 };
        let _ = app.emit("update-progress", UpdateProgress { downloaded, total, percentage });
    }

    let app_dir = std::env::current_exe()
        .map_err(|e| format!("获取应用路径失败: {}", e))?
        .parent()
        .ok_or("无法获取应用目录")?
        .to_path_buf();

    let temp_dir = app_dir.join(".update_temp");
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).ok();
    }
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;

    let archive_path = temp_dir.join("update_archive");
    std::fs::write(&archive_path, &bytes)
        .map_err(|e| format!("写入下载文件失败: {}", e))?;

    if download_url.ends_with(".zip") {
        extract_zip(&archive_path, &temp_dir)?;
    } else if download_url.ends_with(".tar.gz") {
        return Err("tar.gz 格式暂不支持自动更新，请手动下载".to_string());
    } else {
        return Err("不支持的文件格式".to_string());
    }

    replace_binaries(&temp_dir, &app_dir)?;

    std::fs::remove_dir_all(&temp_dir).ok();

    Ok("更新完成，请重启应用".to_string())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("打开 zip 失败: {}", e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {}", e))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("读取 zip 条目失败: {}", e))?;
        if entry.is_file() {
            let name = entry.name().to_string();
            let file_name = std::path::Path::new(&name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(name);
            let out_path = dest.join(&file_name);
            let mut out_file = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败: {}", e))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("解压文件失败: {}", e))?;
        }
    }
    Ok(())
}

fn replace_binaries(temp_dir: &Path, app_dir: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(temp_dir).map_err(|e| format!("读取临时目录失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
        let file_name = entry.file_name().to_string_lossy().to_string();

        if file_name == "update_archive" {
            continue;
        }

        let target = app_dir.join(&file_name);

        if target.exists() {
            let backup = app_dir.join(format!("{}.bak", &file_name));
            std::fs::copy(&target, &backup).ok();
        }

        #[cfg(windows)]
        {
            if target.exists() {
                let old_name = app_dir.join(format!("{}.old", &file_name));
                std::fs::rename(&target, &old_name).ok();
            }
        }

        std::fs::copy(entry.path(), &target)
            .map_err(|e| format!("替换文件失败 {}: {}", file_name, e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !file_name.contains('.') || file_name.ends_with(".exe") {
                if let Ok(meta) = std::fs::metadata(&target) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&target, perms).ok();
                }
            }
        }

        log::info!("已更新: {}", file_name);
    }

    Ok(())
}
