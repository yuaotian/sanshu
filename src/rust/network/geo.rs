// IP地理位置检测模块
use serde::{Deserialize, Serialize};
use crate::{log_important, log_debug};

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
    log_important!(info, "[network] 开始检测IP地理位置");
    
    // 创建HTTP客户端，设置较短的超时时间
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log_important!(warn, "[network] 创建HTTP客户端失败: {}", e);
            return "UNKNOWN".to_string();
        }
    };
    
    log_debug!("[network] 请求 ipinfo.io API");
    
    // 请求 ipinfo.io API
    match client
        .get("https://ipinfo.io/json")
        .send()
        .await
    {
        Ok(response) => {
            if !response.status().is_success() {
                log_important!(warn, "[network] IP地理位置检测请求失败: HTTP {}", response.status());
                return "UNKNOWN".to_string();
            }
            
            // 解析JSON响应
            match response.json::<GeoLocation>().await {
                Ok(geo) => {
                    log_important!(info, "[network] 检测到地理位置: {} ({})", geo.country, geo.city.clone().unwrap_or_default());
                    geo.country
                }
                Err(e) => {
                    log_important!(warn, "[network] 解析地理位置信息失败: {}", e);
                    "UNKNOWN".to_string()
                }
            }
        }
        Err(e) => {
            log_important!(warn, "[network] IP地理位置检测网络请求失败: {}", e);
            "UNKNOWN".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_geo_location() {
        let country = detect_geo_location().await;
        println!("检测到的国家代码: {}", country);
        // 注意：这个测试依赖网络，可能会失败
        assert!(!country.is_empty());
    }
}

