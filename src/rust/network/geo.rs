// IP地理位置检测模块
use crate::{log_debug, log_important};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

const GEO_CACHE_FILE: &str = "geo_location_cache.json";
const GEO_CACHE_TTL_HOURS: i64 = 24;

static MEMORY_CACHE: Lazy<Mutex<Option<(Instant, GeoLocation)>>> = Lazy::new(|| Mutex::new(None));

/// IP地理位置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub ip: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: String,
    pub loc: Option<String>,
    pub org: Option<String>,
    pub postal: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeoLocationCache {
    updated_at: String,
    value: GeoLocation,
}

/// 检测当前IP的地理位置
///
/// 使用 ipinfo.io API 检测IP地理位置
/// 返回国家代码（如 "CN", "US" 等）
///
/// # 错误处理
/// - 网络请求失败时返回 "UNKNOWN"
/// - 解析失败时返回 "UNKNOWN"
/// - 超时设置为 5 秒
pub async fn detect_geo_location() -> String {
    detect_geo_location_full().await.country
}

pub async fn detect_geo_location_full() -> GeoLocation {
    if let Ok(cache) = MEMORY_CACHE.lock() {
        if let Some((created_at, value)) = cache.as_ref() {
            if created_at.elapsed() < Duration::from_secs(60 * 60 * 24) {
                return value.clone();
            }
        }
    }
    if let Some(value) = read_geo_cache(false) {
        remember(value.clone());
        return value;
    }

    log_important!(info, "[network] 开始检测 IP 地理位置");

    // 创建HTTP客户端，设置较短的超时时间
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log_important!(warn, "[network] 创建HTTP客户端失败: {}", e);
            return stale_or_unknown();
        }
    };

    log_debug!("[network] 请求 ipinfo.io API");

    // 请求 ipinfo.io API
    match client.get("https://ipinfo.io/json").send().await {
        Ok(response) => {
            if !response.status().is_success() {
                log_important!(
                    warn,
                    "[network] IP地理位置检测请求失败: HTTP {}",
                    response.status()
                );
                return stale_or_unknown();
            }

            // 解析JSON响应
            match response.json::<GeoLocation>().await {
                Ok(geo) => {
                    log_important!(
                        info,
                        "[network] 检测到地理位置: {} ({})",
                        geo.country,
                        geo.city.clone().unwrap_or_default()
                    );
                    let _ = write_geo_cache(&geo);
                    remember(geo.clone());
                    geo
                }
                Err(e) => {
                    log_important!(warn, "[network] 解析地理位置信息失败: {}", e);
                    stale_or_unknown()
                }
            }
        }
        Err(e) => {
            log_important!(warn, "[network] IP地理位置检测网络请求失败: {}", e);
            stale_or_unknown()
        }
    }
}

fn unknown_location() -> GeoLocation {
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

fn remember(value: GeoLocation) {
    if let Ok(mut cache) = MEMORY_CACHE.lock() {
        *cache = Some((Instant::now(), value));
    }
}

fn stale_or_unknown() -> GeoLocation {
    let value = read_geo_cache(true).unwrap_or_else(unknown_location);
    remember(value.clone());
    value
}

fn read_geo_cache(allow_stale: bool) -> Option<GeoLocation> {
    let raw = fs::read_to_string(geo_cache_path()?).ok()?;
    let cache = serde_json::from_str::<GeoLocationCache>(&raw).ok()?;
    if allow_stale {
        return Some(cache.value);
    }
    let updated_at = chrono::DateTime::parse_from_rfc3339(&cache.updated_at).ok()?;
    (chrono::Utc::now().signed_duration_since(updated_at.with_timezone(&chrono::Utc))
        <= chrono::Duration::hours(GEO_CACHE_TTL_HOURS))
    .then_some(cache.value)
}

fn write_geo_cache(value: &GeoLocation) -> Result<(), String> {
    let path = geo_cache_path().ok_or_else(|| "无法确定地理位置缓存目录".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建地理位置缓存目录失败: {}", error))?;
    }
    let cache = GeoLocationCache {
        updated_at: chrono::Utc::now().to_rfc3339(),
        value: value.clone(),
    };
    let raw = serde_json::to_vec_pretty(&cache)
        .map_err(|error| format!("序列化地理位置缓存失败: {}", error))?;
    let part = path.with_extension(format!("json.{}.part", std::process::id()));
    fs::write(&part, raw).map_err(|error| format!("写入地理位置缓存失败: {}", error))?;
    if path.exists() {
        fs::remove_file(&path).map_err(|error| format!("替换地理位置缓存失败: {}", error))?;
    }
    fs::rename(&part, &path).map_err(|error| format!("原子写入地理位置缓存失败: {}", error))
}

fn geo_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("sanshu").join(GEO_CACHE_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "依赖外部 ipinfo.io 服务"]
    async fn test_detect_geo_location() {
        let country = detect_geo_location().await;
        println!("检测到的国家代码: {}", country);
        // 注意：这个测试依赖网络，可能会失败
        assert!(!country.is_empty());
    }
}
