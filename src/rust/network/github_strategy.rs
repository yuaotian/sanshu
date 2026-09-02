// GitHub 访问策略模块：直连、代理站轮询、本地代理兜底与测速缓存
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::ProxyConfig;
use crate::network::geo::detect_geo_location;
use crate::network::proxy::{ProxyDetector, ProxyInfo, ProxyType};
use crate::{log_debug, log_important};

pub const LATEST_RELEASE_API_URL: &str =
    "https://api.github.com/repos/yuaotian/sanshu/releases/latest";
pub const ANNOUNCEMENT_RAW_URL: &str =
    "https://raw.githubusercontent.com/yuaotian/sanshu/refs/heads/main/announcements/latest.json";

const CACHE_FILE_NAME: &str = "github_proxy_cache.json";
const USER_AGENT: &str = concat!("sanshu/", env!("CARGO_PKG_VERSION"));
const DIRECT_TIMEOUT_SECS: u64 = 8;
const CN_DIRECT_TIMEOUT_SECS: u64 = 3;
const MIRROR_DOWNLOAD_TIMEOUT_SECS: u64 = 300;
const DOWNLOAD_TIMEOUT_SECS: u64 = 1_800;
const PROXY_PROBE_TIMEOUT_SECS: u64 = 4;
const CONNECT_TIMEOUT_SECS: u64 = 8;
const CACHE_TTL_HOURS: i64 = 24;
const LOCAL_PROXY_CACHE_TTL_SECS: u64 = 10 * 60;

static LOCAL_PROXY_CACHE: Lazy<Mutex<Option<(Instant, Option<ProxyInfo>)>>> =
    Lazy::new(|| Mutex::new(None));

const GITHUB_PROXY_PREFIXES: &[&str] = &[
    "https://wget.la/",
    "https://rapidgit.jjda.de5.net/",
    "https://fastgit.cc/",
    "https://gitproxy.mrhjx.cn/",
    "https://github.boki.moe/",
    "https://github.ednovas.xyz/",
];

#[derive(Debug, Clone, Copy)]
pub enum GitHubResourceKind {
    Api,
    Raw,
    ReleaseAsset,
}

impl GitHubResourceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Raw => "raw",
            Self::ReleaseAsset => "release",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRouteSummary {
    pub label: String,
    pub url: String,
    pub used_mirror: bool,
    pub using_local_proxy: bool,
    pub proxy_host: Option<String>,
    pub proxy_port: Option<u16>,
    pub proxy_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GitHubJsonResult {
    pub value: serde_json::Value,
    pub route: GitHubRouteSummary,
}

#[derive(Debug, Clone)]
pub struct GitHubDownloadProgress {
    pub chunk_length: usize,
    pub content_length: Option<u64>,
    pub downloaded: u64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubProxyProbe {
    pub proxy_prefix: String,
    #[serde(default)]
    pub api_ok: bool,
    #[serde(default)]
    pub raw_ok: bool,
    #[serde(default)]
    pub release_ok: bool,
    pub latency_ms: Option<u128>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubProxyCache {
    pub updated_at: String,
    pub probes: Vec<GitHubProxyProbe>,
}

#[derive(Debug, Clone)]
struct RequestCandidate {
    label: String,
    url: String,
    proxy: Option<ProxyInfo>,
    used_mirror: bool,
    connect_timeout_secs: u64,
    timeout_secs: u64,
}

impl RequestCandidate {
    fn route_summary(&self) -> GitHubRouteSummary {
        GitHubRouteSummary {
            label: self.label.clone(),
            url: self.url.clone(),
            used_mirror: self.used_mirror,
            using_local_proxy: self.proxy.is_some(),
            proxy_host: self.proxy.as_ref().map(|proxy| proxy.host.clone()),
            proxy_port: self.proxy.as_ref().map(|proxy| proxy.port),
            proxy_type: self
                .proxy
                .as_ref()
                .map(|proxy| proxy.proxy_type.to_string()),
        }
    }
}

pub async fn fetch_latest_release_with_strategy(
    proxy_config: &ProxyConfig,
) -> Result<GitHubJsonResult, String> {
    fetch_json_with_strategy(
        LATEST_RELEASE_API_URL,
        GitHubResourceKind::Api,
        proxy_config,
    )
    .await
}

pub async fn fetch_announcement_with_strategy(
    proxy_config: &ProxyConfig,
) -> Result<GitHubJsonResult, String> {
    fetch_json_with_strategy(ANNOUNCEMENT_RAW_URL, GitHubResourceKind::Raw, proxy_config).await
}

pub async fn fetch_json_with_strategy(
    url: &str,
    kind: GitHubResourceKind,
    proxy_config: &ProxyConfig,
) -> Result<GitHubJsonResult, String> {
    // JSON 会直接影响版本判断与公告渲染，只走官方域名直连或本地代理。
    let candidates =
        build_candidates(url, kind, proxy_config, DIRECT_TIMEOUT_SECS, false, false).await;
    let mut errors = Vec::new();

    for candidate in candidates {
        match send_get(&candidate).await {
            Ok(response) => match response.json::<serde_json::Value>().await {
                Ok(value) => {
                    log_important!(
                        info,
                        "[github_strategy] {} JSON 获取成功: route={}",
                        kind.label(),
                        candidate.label
                    );
                    return Ok(GitHubJsonResult {
                        value,
                        route: candidate.route_summary(),
                    });
                }
                Err(e) => errors.push(format!("{} JSON解析失败: {}", candidate.label, e)),
            },
            Err(e) => errors.push(format!("{} 请求失败: {}", candidate.label, e)),
        }
    }

    Err(format!(
        "GitHub {} 请求全部失败: {}",
        kind.label(),
        errors.join(" | ")
    ))
}

pub async fn download_with_strategy(
    url: &str,
    target_path: &Path,
    proxy_config: &ProxyConfig,
) -> Result<GitHubRouteSummary, String> {
    download_with_strategy_with_progress(url, target_path, proxy_config, |_| {}).await
}

pub async fn download_with_strategy_with_progress<F>(
    url: &str,
    target_path: &Path,
    proxy_config: &ProxyConfig,
    on_progress: F,
) -> Result<GitHubRouteSummary, String>
where
    F: FnMut(GitHubDownloadProgress) + Send,
{
    download_verified_with_strategy_with_progress(url, target_path, proxy_config, None, on_progress)
        .await
}

pub async fn download_verified_with_strategy_with_progress<F>(
    url: &str,
    target_path: &Path,
    proxy_config: &ProxyConfig,
    expected_sha256: Option<&str>,
    on_progress: F,
) -> Result<GitHubRouteSummary, String>
where
    F: FnMut(GitHubDownloadProgress) + Send,
{
    download_verified_with_strategy_with_progress_and_cancel(
        url,
        target_path,
        proxy_config,
        expected_sha256,
        on_progress,
        || false,
    )
    .await
}

pub async fn download_verified_with_strategy_with_progress_and_cancel<F, C>(
    url: &str,
    target_path: &Path,
    proxy_config: &ProxyConfig,
    expected_sha256: Option<&str>,
    on_progress: F,
    should_cancel: C,
) -> Result<GitHubRouteSummary, String>
where
    F: FnMut(GitHubDownloadProgress) + Send,
    C: FnMut() -> bool + Send,
{
    // 代理站内容只有在调用方提供 SHA-256 时才进入候选，避免把可执行更新交给无信任根的中转站。
    let allow_mirrors = expected_sha256.is_some();
    let mut on_progress = on_progress;
    download_verified_with_strategy_inner(
        url,
        target_path,
        proxy_config,
        expected_sha256,
        move |progress, _route| on_progress(progress),
        should_cancel,
        allow_mirrors,
        false,
    )
    .await
}

/// 非 GitHub 固定资产只使用官方地址与本地代理，并把本地代理作为首选路线。
pub async fn download_verified_with_direct_or_local_proxy_with_progress_and_cancel<F, C>(
    url: &str,
    target_path: &Path,
    proxy_config: &ProxyConfig,
    expected_sha256: Option<&str>,
    on_progress: F,
    should_cancel: C,
) -> Result<GitHubRouteSummary, String>
where
    F: FnMut(GitHubDownloadProgress, &GitHubRouteSummary) + Send,
    C: FnMut() -> bool + Send,
{
    download_verified_with_strategy_inner(
        url,
        target_path,
        proxy_config,
        expected_sha256,
        on_progress,
        should_cancel,
        false,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn download_verified_with_strategy_inner<F, C>(
    url: &str,
    target_path: &Path,
    proxy_config: &ProxyConfig,
    expected_sha256: Option<&str>,
    mut on_progress: F,
    mut should_cancel: C,
    allow_mirrors: bool,
    prefer_local_proxy: bool,
) -> Result<GitHubRouteSummary, String>
where
    F: FnMut(GitHubDownloadProgress, &GitHubRouteSummary) + Send,
    C: FnMut() -> bool + Send,
{
    let candidates = build_candidates(
        url,
        GitHubResourceKind::ReleaseAsset,
        proxy_config,
        DOWNLOAD_TIMEOUT_SECS,
        allow_mirrors,
        prefer_local_proxy,
    )
    .await;
    let mut errors = Vec::new();
    let part_path = partial_path(target_path);

    for candidate in candidates {
        if should_cancel() {
            return Err("下载已取消".to_string());
        }
        match send_download_get(&candidate, &part_path).await {
            Ok(response) => {
                let route = candidate.route_summary();
                let mut route_progress = |progress| on_progress(progress, &route);
                if let Err(e) = stream_response_to_file(
                    response,
                    &part_path,
                    &mut route_progress,
                    &mut should_cancel,
                )
                .await
                {
                    if should_cancel() {
                        return Err("下载已取消".to_string());
                    }
                    errors.push(format!("{} 写入失败: {}", candidate.label, e));
                    continue;
                }

                if let Some(expected) = expected_sha256 {
                    match sha256_file(&part_path) {
                        Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
                        Ok(actual) => {
                            let _ = fs::remove_file(&part_path);
                            errors.push(format!(
                                "{} SHA256 不匹配: expected={}, actual={}",
                                candidate.label, expected, actual
                            ));
                            continue;
                        }
                        Err(error) => {
                            errors.push(format!("{} SHA256 计算失败: {}", candidate.label, error));
                            continue;
                        }
                    }
                }
                if target_path.exists() {
                    fs::remove_file(target_path).map_err(|error| {
                        format!("移除旧下载文件 {} 失败: {}", target_path.display(), error)
                    })?;
                }
                fs::rename(&part_path, target_path).map_err(|error| {
                    format!("原子替换下载文件 {} 失败: {}", target_path.display(), error)
                })?;

                log_important!(
                    info,
                    "[github_strategy] Release 下载成功: route={}, target={}",
                    candidate.label,
                    target_path.display()
                );
                return Ok(route);
            }
            Err(e) => errors.push(format!("{} 下载失败: {}", candidate.label, e)),
        }
    }

    Err(format!("Release 下载全部失败: {}", errors.join(" | ")))
}

pub async fn refresh_github_proxy_cache() -> Result<GitHubProxyCache, String> {
    if let Some(cache) = read_proxy_cache() {
        log_debug!("[github_strategy] GitHub 代理站测速缓存仍在有效期内");
        return Ok(cache);
    }
    log_important!(info, "[github_strategy] 开始刷新 GitHub 代理站测速缓存");

    let release_asset_url = latest_release_asset_url_direct().await.ok();
    let mut probes = Vec::new();
    for prefix in GITHUB_PROXY_PREFIXES {
        let started = Instant::now();
        let api_url = mirror_url(prefix, LATEST_RELEASE_API_URL);
        let raw_url = mirror_url(prefix, ANNOUNCEMENT_RAW_URL);
        let release_url = release_asset_url
            .as_deref()
            .map(|url| mirror_url(prefix, url));

        let api_result = probe_mirror_json(&api_url).await;
        let raw_result = probe_mirror_json(&raw_url).await;
        let release_result = match release_url {
            Some(url) => probe_mirror_asset(&url).await,
            None => Err("未取得最新 Release 资产 URL".to_string()),
        };
        let latency_ms = started.elapsed().as_millis();
        let api_ok = api_result.is_ok();
        let raw_ok = raw_result.is_ok();
        let release_ok = release_result.is_ok();
        let error = if api_ok || raw_ok || release_ok {
            None
        } else {
            Some(format!(
                "api={}, raw={}, release={}",
                api_result.unwrap_err(),
                raw_result.unwrap_err(),
                release_result.unwrap_err()
            ))
        };

        probes.push(GitHubProxyProbe {
            proxy_prefix: (*prefix).to_string(),
            api_ok,
            raw_ok,
            release_ok,
            latency_ms: Some(latency_ms),
            error,
        });
    }

    probes.sort_by_key(|probe| {
        (
            !(probe.api_ok || probe.raw_ok || probe.release_ok),
            probe.latency_ms.unwrap_or(u128::MAX),
        )
    });

    let cache = GitHubProxyCache {
        updated_at: chrono::Utc::now().to_rfc3339(),
        probes,
    };
    write_proxy_cache(&cache)?;

    log_important!(info, "[github_strategy] GitHub 代理站测速缓存刷新完成");
    Ok(cache)
}

async fn build_candidates(
    original_url: &str,
    kind: GitHubResourceKind,
    proxy_config: &ProxyConfig,
    timeout_secs: u64,
    allow_mirrors: bool,
    prefer_local_proxy: bool,
) -> Vec<RequestCandidate> {
    let country = detect_geo_location().await;
    let direct_timeout = if country == "CN" || country == "UNKNOWN" {
        CN_DIRECT_TIMEOUT_SECS
    } else {
        timeout_secs
    };

    let direct = RequestCandidate {
        label: format!("github-direct-{}", country),
        url: original_url.to_string(),
        proxy: None,
        used_mirror: false,
        connect_timeout_secs: CONNECT_TIMEOUT_SECS,
        timeout_secs: direct_timeout,
    };

    let local_proxy = detect_local_proxy(proxy_config)
        .await
        .map(|proxy| RequestCandidate {
            label: format!("local-proxy:{}", proxy.to_url()),
            url: original_url.to_string(),
            proxy: Some(proxy),
            used_mirror: false,
            connect_timeout_secs: CONNECT_TIMEOUT_SECS,
            timeout_secs,
        });

    let mut candidates = order_primary_candidates(direct, local_proxy, prefer_local_proxy);

    if allow_mirrors {
        for prefix in sorted_proxy_prefixes(kind) {
            candidates.push(RequestCandidate {
                label: format!("github-proxy:{}", prefix.trim_end_matches('/')),
                url: mirror_url(&prefix, original_url),
                proxy: None,
                used_mirror: true,
                connect_timeout_secs: CONNECT_TIMEOUT_SECS,
                timeout_secs: timeout_secs.min(MIRROR_DOWNLOAD_TIMEOUT_SECS),
            });
        }
    }

    log_debug!(
        "[github_strategy] 候选路由: kind={}, count={}",
        kind.label(),
        candidates.len()
    );
    candidates
}

fn order_primary_candidates(
    direct: RequestCandidate,
    local_proxy: Option<RequestCandidate>,
    prefer_local_proxy: bool,
) -> Vec<RequestCandidate> {
    let mut candidates = Vec::with_capacity(1 + if local_proxy.is_some() { 1 } else { 0 });
    if prefer_local_proxy {
        if let Some(proxy) = local_proxy {
            candidates.push(proxy);
        }
        candidates.push(direct);
    } else {
        candidates.push(direct);
        if let Some(proxy) = local_proxy {
            candidates.push(proxy);
        }
    }
    candidates
}

async fn detect_local_proxy(proxy_config: &ProxyConfig) -> Option<ProxyInfo> {
    if proxy_config.enabled && !proxy_config.auto_detect {
        let proxy_type = match proxy_config.proxy_type.as_str() {
            "socks5" => ProxyType::Socks5,
            _ => ProxyType::Http,
        };
        return Some(ProxyInfo::new(
            proxy_type,
            proxy_config.host.clone(),
            proxy_config.port,
        ));
    }

    if proxy_config.auto_detect || proxy_config.enabled {
        if let Ok(cache) = LOCAL_PROXY_CACHE.lock() {
            if let Some((created_at, proxy)) = cache.as_ref() {
                if created_at.elapsed() < Duration::from_secs(LOCAL_PROXY_CACHE_TTL_SECS) {
                    return proxy.clone();
                }
            }
        }
        let detected = ProxyDetector::detect_available_proxy().await;
        if let Ok(mut cache) = LOCAL_PROXY_CACHE.lock() {
            *cache = Some((Instant::now(), detected.clone()));
        }
        return detected;
    }

    None
}

fn sorted_proxy_prefixes(kind: GitHubResourceKind) -> Vec<String> {
    let mut prefixes = Vec::new();
    let mut seen = HashSet::new();

    if let Some(cache) = read_proxy_cache() {
        for probe in cache.probes {
            let ok_for_kind = match kind {
                GitHubResourceKind::Api => probe.api_ok,
                GitHubResourceKind::Raw => probe.raw_ok,
                GitHubResourceKind::ReleaseAsset => probe.release_ok,
            };
            if ok_for_kind && seen.insert(probe.proxy_prefix.clone()) {
                prefixes.push(probe.proxy_prefix);
            }
        }
    }

    for prefix in GITHUB_PROXY_PREFIXES {
        let prefix = (*prefix).to_string();
        if seen.insert(prefix.clone()) {
            prefixes.push(prefix);
        }
    }

    prefixes
}

fn mirror_url(prefix: &str, original_url: &str) -> String {
    format!("{}/{}", prefix.trim_end_matches('/'), original_url)
}

async fn probe_mirror_json(url: &str) -> Result<(), String> {
    let candidate = RequestCandidate {
        label: url.to_string(),
        url: url.to_string(),
        proxy: None,
        used_mirror: true,
        connect_timeout_secs: PROXY_PROBE_TIMEOUT_SECS,
        timeout_secs: PROXY_PROBE_TIMEOUT_SECS,
    };
    let response = send_get(&candidate).await?;
    let _ = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn probe_mirror_asset(url: &str) -> Result<(), String> {
    let candidate = RequestCandidate {
        label: url.to_string(),
        url: url.to_string(),
        proxy: None,
        used_mirror: true,
        connect_timeout_secs: PROXY_PROBE_TIMEOUT_SECS,
        timeout_secs: PROXY_PROBE_TIMEOUT_SECS,
    };
    let client = create_strategy_client(&candidate)?;
    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Range", "bytes=0-0")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", response.status()))
    }
}

async fn latest_release_asset_url_direct() -> Result<String, String> {
    let candidate = RequestCandidate {
        label: "github-direct-probe".to_string(),
        url: LATEST_RELEASE_API_URL.to_string(),
        proxy: None,
        used_mirror: false,
        connect_timeout_secs: CONNECT_TIMEOUT_SECS,
        timeout_secs: DIRECT_TIMEOUT_SECS,
    };
    let value = send_get(&candidate)
        .await?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    value["assets"]
        .as_array()
        .and_then(|assets| assets.first())
        .and_then(|asset| asset["browser_download_url"].as_str())
        .map(str::to_string)
        .ok_or_else(|| "最新 Release 没有可探测资产".to_string())
}

async fn send_get(candidate: &RequestCandidate) -> Result<reqwest::Response, String> {
    let client = create_strategy_client(candidate)?;
    let response = client
        .get(&candidate.url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    Ok(response)
}

async fn send_download_get(
    candidate: &RequestCandidate,
    part_path: &Path,
) -> Result<reqwest::Response, String> {
    let client = create_strategy_client(candidate)?;
    let existing = fs::metadata(part_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let mut request = client
        .get(&candidate.url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/octet-stream");
    if existing > 0 {
        request = request.header("Range", format!("bytes={}-", existing));
    }
    let mut response = request.send().await.map_err(|error| error.to_string())?;
    if existing > 0 && response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        fs::remove_file(part_path).map_err(|error| format!("清理不可续传分片失败: {}", error))?;
        response = client
            .get(&candidate.url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/octet-stream")
            .send()
            .await
            .map_err(|error| error.to_string())?;
    }
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    Ok(response)
}

fn create_strategy_client(candidate: &RequestCandidate) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(candidate.connect_timeout_secs))
        .timeout(Duration::from_secs(candidate.timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(10));

    if let Some(proxy) = &candidate.proxy {
        let proxy_url = proxy.to_url();
        let reqwest_proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| format!("创建代理失败 {}: {}", proxy_url, e))?;
        builder = builder.proxy(reqwest_proxy);
    }

    builder
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {}", e))
}

async fn stream_response_to_file(
    response: reqwest::Response,
    target_path: &Path,
    on_progress: &mut (impl FnMut(GitHubDownloadProgress) + Send),
    should_cancel: &mut (impl FnMut() -> bool + Send),
) -> Result<(), String> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建下载目录失败 {}: {}", parent.display(), e))?;
    }

    let existing = fs::metadata(target_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let append = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let offset = if append { existing } else { 0 };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(target_path)
        .map_err(|e| format!("创建下载文件失败 {}: {}", target_path.display(), e))?;
    let content_length = response.content_length().map(|length| length + offset);
    let mut downloaded = offset;
    let mut stream = response.bytes_stream();

    // 中文说明：每个候选路由开始下载时先发送 0% 进度，让前端可感知重试切换。
    on_progress(GitHubDownloadProgress {
        chunk_length: 0,
        content_length,
        downloaded,
        percentage: 0.0,
    });

    while let Some(chunk) = stream.next().await {
        if should_cancel() {
            return Err("下载已取消".to_string());
        }
        let chunk = chunk.map_err(|e| format!("读取下载数据失败: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("写入下载文件失败: {}", e))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        on_progress(GitHubDownloadProgress {
            chunk_length: chunk.len(),
            content_length,
            downloaded,
            percentage: calculate_download_percentage(downloaded, content_length),
        });
    }

    // 中文说明：content-length 缺失时无法实时计算百分比，落盘结束后统一补发 100%。
    on_progress(GitHubDownloadProgress {
        chunk_length: 0,
        content_length: content_length.or(Some(downloaded)),
        downloaded,
        percentage: 100.0,
    });

    Ok(())
}

fn calculate_download_percentage(downloaded: u64, content_length: Option<u64>) -> f64 {
    match content_length {
        Some(total) if total > 0 => ((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 100.0),
        _ => 0.0,
    }
}

fn read_proxy_cache() -> Option<GitHubProxyCache> {
    let path = proxy_cache_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let cache = serde_json::from_str::<GitHubProxyCache>(&raw).ok()?;
    let updated_at = chrono::DateTime::parse_from_rfc3339(&cache.updated_at).ok()?;
    (chrono::Utc::now().signed_duration_since(updated_at.with_timezone(&chrono::Utc))
        <= chrono::Duration::hours(CACHE_TTL_HOURS))
    .then_some(cache)
}

fn write_proxy_cache(cache: &GitHubProxyCache) -> Result<(), String> {
    let path = proxy_cache_path().ok_or_else(|| "无法确定 GitHub 代理缓存路径".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 GitHub 代理缓存目录失败: {}", e))?;
    }
    let raw = serde_json::to_string_pretty(cache)
        .map_err(|e| format!("序列化 GitHub 代理缓存失败: {}", e))?;
    let part = path.with_extension(format!("json.{}.part", std::process::id()));
    fs::write(&part, raw)
        .map_err(|e| format!("写入 GitHub 代理缓存失败 {}: {}", part.display(), e))?;
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| format!("替换 GitHub 代理缓存失败 {}: {}", path.display(), e))?;
    }
    fs::rename(&part, &path)
        .map_err(|e| format!("原子写入 GitHub 代理缓存失败 {}: {}", path.display(), e))
}

fn proxy_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("sanshu").join(CACHE_FILE_NAME))
}

fn partial_path(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    target_path.with_file_name(format!("{}.part", file_name))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("打开 {} 失败: {}", path.display(), error))?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("读取 {} 失败: {}", path.display(), error))?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }
    Ok(hex::encode(context.finish().as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn test_candidate(label: &str, proxy: Option<ProxyInfo>) -> RequestCandidate {
        RequestCandidate {
            label: label.to_string(),
            url: "https://example.invalid/model.onnx".to_string(),
            proxy,
            used_mirror: false,
            connect_timeout_secs: 1,
            timeout_secs: 1,
        }
    }

    #[test]
    fn primary_candidate_order_respects_local_proxy_preference() {
        let proxy = ProxyInfo::new(ProxyType::Http, "127.0.0.1".to_string(), 7890);
        let preferred = order_primary_candidates(
            test_candidate("direct", None),
            Some(test_candidate("proxy", Some(proxy.clone()))),
            true,
        );
        assert_eq!(preferred[0].label, "proxy");
        assert_eq!(preferred[1].label, "direct");

        let fallback = order_primary_candidates(
            test_candidate("direct", None),
            Some(test_candidate("proxy", Some(proxy))),
            false,
        );
        assert_eq!(fallback[0].label, "direct");
        assert_eq!(fallback[1].label, "proxy");
    }

    #[test]
    fn partial_download_uses_sibling_part_file() {
        let target = Path::new("updates/sanshu.zip");
        assert_eq!(
            partial_path(target),
            PathBuf::from("updates/sanshu.zip.part")
        );
    }

    #[test]
    fn legacy_proxy_cache_defaults_api_capability_to_false() {
        let raw = r#"{
            "updated_at":"2026-08-31T00:00:00Z",
            "probes":[{
                "proxy_prefix":"https://mirror.invalid/",
                "raw_ok":true,
                "release_ok":false,
                "latency_ms":12,
                "error":null
            }]
        }"#;
        let cache: GitHubProxyCache = serde_json::from_str(raw).expect("旧缓存应保持兼容");
        assert!(!cache.probes[0].api_ok);
        assert!(cache.probes[0].raw_ok);
    }

    #[tokio::test]
    async fn range_not_satisfiable_restarts_same_route_without_range() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("应启动本地下载测试服务");
        let address = listener.local_addr().expect("应读取本地监听地址");
        let server = thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().expect("应接收本地测试请求");
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let count = stream.read(&mut buffer).expect("应读取本地测试请求");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                if attempt == 0 {
                    assert!(request.contains("range: bytes=5-"));
                    stream
                        .write_all(
                            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .expect("应返回 416");
                } else {
                    assert!(!request.contains("range:"));
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfresh",
                        )
                        .expect("应返回完整文件");
                }
            }
        });

        let directory = tempfile::tempdir().expect("应创建下载恢复测试目录");
        let part = directory.path().join("asset.zip.part");
        fs::write(&part, b"stale").expect("应写入旧分片");
        let candidate = RequestCandidate {
            label: "local-range-test".to_string(),
            url: format!("http://{}/asset.zip", address),
            proxy: None,
            used_mirror: false,
            connect_timeout_secs: 2,
            timeout_secs: 2,
        };

        let response = send_download_get(&candidate, &part)
            .await
            .expect("416 后应重新请求完整文件");
        assert_eq!(response.bytes().await.expect("应读取响应"), b"fresh"[..]);
        assert!(!part.exists());
        server.join().expect("本地测试服务应正常退出");
    }
}
