// Iconfont API 封装
// 负责与 iconfont.cn 的 API 进行通信

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use super::types::{
    IconItem, IconSearchRequest, IconSearchResult,
    IconfontApiResponse, IconfontIcon,
};
use crate::log_debug;

// ============ 常量定义 ============

/// Iconfont 搜索 API 端点
const ICONFONT_SEARCH_API: &str = "https://www.iconfont.cn/api/icon/search.json";

/// 默认缓存过期时间（30分钟）
const DEFAULT_CACHE_EXPIRY_SECS: u64 = 30 * 60;

/// HTTP 请求超时时间
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// 最大重试次数
const MAX_RETRIES: usize = 3;

// ============ 缓存结构 ============

/// 缓存条目
struct CacheEntry {
    /// 缓存的搜索结果
    result: IconSearchResult,
    /// 缓存创建时间
    created_at: Instant,
}

/// 全局搜索结果缓存
static SEARCH_CACHE: Lazy<RwLock<HashMap<String, CacheEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// 缓存过期时间配置（秒）
static CACHE_EXPIRY_SECS: Lazy<RwLock<u64>> =
    Lazy::new(|| RwLock::new(DEFAULT_CACHE_EXPIRY_SECS));

// ============ HTTP 客户端 ============

/// 创建带有默认配置的 HTTP 客户端
fn create_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| anyhow!("创建 HTTP 客户端失败: {}", e))
}

// ============ 缓存管理 ============

/// 生成缓存键
fn generate_cache_key(request: &IconSearchRequest) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        request.query,
        request.style.as_deref().unwrap_or("all"),
        request.fills.as_deref().unwrap_or("all"),
        request.sort_type.as_deref().unwrap_or("relate"),
        request.page.unwrap_or(1),
        request.page_size.unwrap_or(50),
        request.from_collection.unwrap_or(false),
    )
}

/// 从缓存获取结果
fn get_from_cache(key: &str) -> Option<IconSearchResult> {
    let cache = SEARCH_CACHE.read().ok()?;
    let entry = cache.get(key)?;
    
    // 检查是否过期
    let expiry = CACHE_EXPIRY_SECS.read().ok().map(|e| *e).unwrap_or(DEFAULT_CACHE_EXPIRY_SECS);
    if entry.created_at.elapsed() > Duration::from_secs(expiry) {
        return None;
    }
    
    Some(entry.result.clone())
}

/// 存入缓存
fn put_to_cache(key: String, result: IconSearchResult) {
    if let Ok(mut cache) = SEARCH_CACHE.write() {
        cache.insert(key, CacheEntry {
            result,
            created_at: Instant::now(),
        });
    }
}

/// 获取缓存统计
pub fn get_cache_stats() -> super::types::IconCacheStats {
    let cache = SEARCH_CACHE.read().ok();
    let expiry = CACHE_EXPIRY_SECS.read().ok().map(|e| *e).unwrap_or(DEFAULT_CACHE_EXPIRY_SECS);
    
    match cache {
        Some(cache) => {
            let total = cache.len();
            let valid = cache.values()
                .filter(|e| e.created_at.elapsed() < Duration::from_secs(expiry))
                .count();
            
            super::types::IconCacheStats {
                total_entries: total,
                valid_entries: valid,
                expired_entries: total.saturating_sub(valid),
                cache_expiry_minutes: expiry / 60,
                memory_usage_bytes: None,
            }
        }
        None => super::types::IconCacheStats {
            total_entries: 0,
            valid_entries: 0,
            expired_entries: 0,
            cache_expiry_minutes: expiry / 60,
            memory_usage_bytes: None,
        }
    }
}

/// 清空缓存
pub fn clear_cache(expired_only: bool) -> super::types::ClearCacheResult {
    let mut cleared = 0;
    let mut remaining = 0;
    
    if let Ok(mut cache) = SEARCH_CACHE.write() {
        if expired_only {
            let expiry = CACHE_EXPIRY_SECS.read().ok().map(|e| *e).unwrap_or(DEFAULT_CACHE_EXPIRY_SECS);
            let original_len = cache.len();
            cache.retain(|_, entry| {
                entry.created_at.elapsed() < Duration::from_secs(expiry)
            });
            remaining = cache.len();
            cleared = original_len.saturating_sub(remaining);
        } else {
            cleared = cache.len();
            cache.clear();
            remaining = 0;
        }
    }
    
    super::types::ClearCacheResult {
        cleared_count: cleared,
        remaining_count: remaining,
    }
}

/// 设置缓存过期时间
pub fn set_cache_expiry_minutes(minutes: u64) {
    if let Ok(mut expiry) = CACHE_EXPIRY_SECS.write() {
        *expiry = minutes * 60;
    }
}

// ============ API 调用 ============

/// 搜索图标
/// 
/// 调用 Iconfont API 搜索图标，支持缓存
pub async fn search_icons(request: IconSearchRequest) -> Result<IconSearchResult> {
    // 参数验证
    if request.query.trim().is_empty() {
        return Err(anyhow!("搜索关键词不能为空"));
    }
    
    // 检查缓存
    let cache_key = generate_cache_key(&request);
    if let Some(cached) = get_from_cache(&cache_key) {
        log_debug!("图标搜索命中缓存: {}", cache_key);
        return Ok(cached);
    }
    
    // 构建请求参数
    let page = request.page.unwrap_or(1);
    let page_size = request.page_size.unwrap_or(50);
    
    let mut params = HashMap::new();
    params.insert("q", request.query.clone());
    params.insert("sortType", request.sort_type.clone().unwrap_or_else(|| "relate".to_string()));
    params.insert("page", page.to_string());
    params.insert("pageSize", page_size.to_string());
    
    if let Some(ref style) = request.style {
        if style != "all" {
            params.insert("sType", style.clone());
        }
    }
    
    if let Some(ref fills) = request.fills {
        if fills != "all" {
            params.insert("fills", fills.clone());
        }
    }
    
    if request.from_collection.unwrap_or(false) {
        params.insert("fromCollection", "1".to_string());
    }
    
    // 执行请求（带重试）
    let result = retry_search_request(&params).await?;
    
    // 解析响应
    let search_result = parse_search_response(result, page, page_size)?;
    
    // 存入缓存
    put_to_cache(cache_key, search_result.clone());
    
    Ok(search_result)
}

/// 带重试的搜索请求
async fn retry_search_request(params: &HashMap<&str, String>) -> Result<IconfontApiResponse> {
    let client = create_http_client()?;
    let mut last_error = None;
    
    for attempt in 0..MAX_RETRIES {
        if attempt > 0 {
            // 指数退避
            let delay = Duration::from_millis(100 * (2_u64.pow(attempt as u32)));
            tokio::time::sleep(delay).await;
            log_debug!("图标搜索重试第 {} 次", attempt + 1);
        }
        
        match execute_search_request(&client, params).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                last_error = Some(e);
            }
        }
    }
    
    Err(last_error.unwrap_or_else(|| anyhow!("搜索请求失败，已达到最大重试次数")))
}

/// 执行单次搜索请求
async fn execute_search_request(
    client: &Client,
    params: &HashMap<&str, String>,
) -> Result<IconfontApiResponse> {
    let response = client
        .post(ICONFONT_SEARCH_API)
        .form(params)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                anyhow!("请求超时")
            } else if e.is_connect() {
                anyhow!("网络连接失败")
            } else {
                anyhow!("请求失败: {}", e)
            }
        })?;
    
    if !response.status().is_success() {
        return Err(anyhow!("API 返回错误状态码: {}", response.status()));
    }
    
    let api_response: IconfontApiResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("解析响应 JSON 失败: {}", e))?;
    
    if api_response.code != 200 {
        return Err(anyhow!(
            "API 返回错误: code={}, message={}",
            api_response.code,
            api_response.message.as_deref().unwrap_or("未知错误")
        ));
    }
    
    Ok(api_response)
}

/// 解析搜索响应
fn parse_search_response(
    response: IconfontApiResponse,
    page: u32,
    page_size: u32,
) -> Result<IconSearchResult> {
    let data = response.data.ok_or_else(|| anyhow!("API 响应缺少 data 字段"))?;
    
    let icons: Vec<IconItem> = data.icons
        .into_iter()
        .map(IconItem::from)
        .collect();
    
    let total = data.count;
    let has_more = page * page_size < total;
    
    Ok(IconSearchResult {
        icons,
        total,
        page,
        page_size,
        has_more,
    })
}

/// 获取图标 SVG 内容
/// 
/// 根据图标 ID 获取 SVG 内容（如果搜索结果中已包含则直接返回）
pub async fn get_icon_svg(id: u64, cached_svg: Option<String>) -> Result<String> {
    // 如果已有缓存的 SVG 内容，直接返回
    if let Some(svg) = cached_svg {
        if !svg.is_empty() {
            return Ok(svg);
        }
    }
    
    // 否则需要单独请求（Iconfont 的图标详情 API）
    // 注意：Iconfont 的搜索结果通常已包含 show_svg 字段，
    // 这里提供备用方案
    let svg_url = format!(
        "https://www.iconfont.cn/api/icon/detail.json?id={}",
        id
    );
    
    let client = create_http_client()?;
    let response = client
        .get(&svg_url)
        .send()
        .await
        .map_err(|e| anyhow!("获取图标详情失败: {}", e))?;
    
    if !response.status().is_success() {
        return Err(anyhow!("获取图标详情失败: {}", response.status()));
    }
    
    // 解析详情响应
    #[derive(serde::Deserialize)]
    struct DetailResponse {
        code: i32,
        data: Option<DetailData>,
    }
    
    #[derive(serde::Deserialize)]
    struct DetailData {
        icon: Option<IconfontIcon>,
    }
    
    let detail: DetailResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("解析图标详情失败: {}", e))?;
    
    if detail.code != 200 {
        return Err(anyhow!("获取图标详情失败: API 返回错误"));
    }
    
    detail.data
        .and_then(|d| d.icon)
        .and_then(|i| i.show_svg)
        .ok_or_else(|| anyhow!("图标 {} 没有 SVG 内容", id))
}

// ============ 辅助函数 ============

/// 构建图标预览 URL
pub fn build_preview_url(id: u64) -> String {
    // Iconfont 的图标预览 URL 格式
    format!(
        "https://at.alicdn.com/t/c/font_{}_preview.svg",
        id
    )
}

/// 构建图标下载 URL
pub fn build_download_url(id: u64, format: &str, size: Option<u32>) -> String {
    match format {
        "png" => {
            let size = size.unwrap_or(64);
            format!(
                "https://www.iconfont.cn/api/icon/downloadIcon?id={}&type=png&size={}",
                id, size
            )
        }
        _ => {
            format!(
                "https://www.iconfont.cn/api/icon/downloadIcon?id={}&type=svg",
                id
            )
        }
    }
}
