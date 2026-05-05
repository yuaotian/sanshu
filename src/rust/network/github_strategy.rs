// GitHub 访问策略模块：直连、代理站轮询、本地代理兜底与测速缓存
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
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
const MIRROR_TIMEOUT_SECS: u64 = 4;
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;
const PROXY_PROBE_TIMEOUT_SECS: u64 = 4;

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
    pub raw_ok: bool,
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
    accept_invalid_certs: bool,
    timeout_secs: u64,
}

impl RequestCandidate {
    fn route_summary(&self) -> GitHubRouteSummary {
        GitHubRouteSummary {
            label: self.label.clone(),
            url: self.url.clone(),
            used_mirror: self.accept_invalid_certs && self.proxy.is_none(),
            using_local_proxy: self.proxy.is_some(),
            proxy_host: self.proxy.as_ref().map(|proxy| proxy.host.clone()),
            proxy_port: self.proxy.as_ref().map(|proxy| proxy.port),
            proxy_type: self.proxy.as_ref().map(|proxy| proxy.proxy_type.to_string()),
        }
    }
}

pub async fn fetch_latest_release_with_strategy(
    proxy_config: &ProxyConfig,
) -> Result<GitHubJsonResult, String> {
    fetch_json_with_strategy(LATEST_RELEASE_API_URL, GitHubResourceKind::Api, proxy_config).await
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
    let candidates = build_candidates(url, kind, proxy_config, DIRECT_TIMEOUT_SECS).await;
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
    mut on_progress: F,
) -> Result<GitHubRouteSummary, String>
where
    F: FnMut(GitHubDownloadProgress) + Send,
{
    let candidates = build_candidates(url, GitHubResourceKind::ReleaseAsset, proxy_config, DOWNLOAD_TIMEOUT_SECS).await;
    let mut errors = Vec::new();

    for candidate in candidates {
        match send_get(&candidate).await {
            Ok(response) => {
                if let Err(e) = stream_response_to_file(response, target_path, &mut on_progress).await {
                    let _ = fs::remove_file(target_path);
                    errors.push(format!("{} 写入失败: {}", candidate.label, e));
                    continue;
                }

                log_important!(
                    info,
                    "[github_strategy] Release 下载成功: route={}, target={}",
                    candidate.label,
                    target_path.display()
                );
                return Ok(candidate.route_summary());
            }
            Err(e) => errors.push(format!("{} 下载失败: {}", candidate.label, e)),
        }
    }

    Err(format!("Release 下载全部失败: {}", errors.join(" | ")))
}

pub async fn refresh_github_proxy_cache() -> Result<GitHubProxyCache, String> {
    log_important!(info, "[github_strategy] 开始刷新 GitHub 代理站测速缓存");

    let mut probes = Vec::new();
    for prefix in GITHUB_PROXY_PREFIXES {
        let started = Instant::now();
        let raw_url = mirror_url(prefix, ANNOUNCEMENT_RAW_URL);
        let release_url = mirror_url(prefix, "https://github.com/yuaotian/sanshu/releases/latest");

        let raw_result = probe_mirror_url(&raw_url).await;
        let release_result = probe_mirror_url(&release_url).await;
        let latency_ms = started.elapsed().as_millis();
        let raw_ok = raw_result.is_ok();
        let release_ok = release_result.is_ok();
        let error = if raw_ok || release_ok {
            None
        } else {
            Some(format!(
                "raw={}, release={}",
                raw_result.unwrap_err(),
                release_result.unwrap_err()
            ))
        };

        probes.push(GitHubProxyProbe {
            proxy_prefix: (*prefix).to_string(),
            raw_ok,
            release_ok,
            latency_ms: Some(latency_ms),
            error,
        });
    }

    probes.sort_by_key(|probe| {
        (
            !(probe.raw_ok || probe.release_ok),
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
) -> Vec<RequestCandidate> {
    let country = detect_geo_location().await;
    let direct_timeout = if country == "CN" || country == "UNKNOWN" {
        CN_DIRECT_TIMEOUT_SECS
    } else {
        timeout_secs
    };

    let mut candidates = Vec::new();
    candidates.push(RequestCandidate {
        label: format!("github-direct-{}", country),
        url: original_url.to_string(),
        proxy: None,
        accept_invalid_certs: false,
        timeout_secs: direct_timeout,
    });

    for prefix in sorted_proxy_prefixes(kind) {
        candidates.push(RequestCandidate {
            label: format!("github-proxy:{}", prefix.trim_end_matches('/')),
            url: mirror_url(&prefix, original_url),
            proxy: None,
            // 中文说明：仅代理站请求允许忽略证书，避免证书过期代理站直接中断整个更新流程。
            accept_invalid_certs: true,
            timeout_secs: MIRROR_TIMEOUT_SECS,
        });
    }

    if let Some(proxy) = detect_local_proxy(proxy_config).await {
        candidates.push(RequestCandidate {
            label: format!("local-proxy:{}", proxy.to_url()),
            url: original_url.to_string(),
            proxy: Some(proxy),
            accept_invalid_certs: false,
            timeout_secs,
        });
    }

    log_debug!(
        "[github_strategy] 候选路由: kind={}, count={}",
        kind.label(),
        candidates.len()
    );
    candidates
}

async fn detect_local_proxy(proxy_config: &ProxyConfig) -> Option<ProxyInfo> {
    if proxy_config.enabled && !proxy_config.auto_detect {
        let proxy_type = match proxy_config.proxy_type.as_str() {
            "socks5" => ProxyType::Socks5,
            _ => ProxyType::Http,
        };
        return Some(ProxyInfo::new(proxy_type, proxy_config.host.clone(), proxy_config.port));
    }

    if proxy_config.auto_detect || proxy_config.enabled {
        return ProxyDetector::detect_available_proxy().await;
    }

    None
}

fn sorted_proxy_prefixes(kind: GitHubResourceKind) -> Vec<String> {
    let mut prefixes = Vec::new();
    let mut seen = HashSet::new();

    if let Some(cache) = read_proxy_cache() {
        for probe in cache.probes {
            let ok_for_kind = match kind {
                GitHubResourceKind::Raw => probe.raw_ok,
                GitHubResourceKind::Api | GitHubResourceKind::ReleaseAsset => probe.release_ok || probe.raw_ok,
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

async fn probe_mirror_url(url: &str) -> Result<(), String> {
    let candidate = RequestCandidate {
        label: url.to_string(),
        url: url.to_string(),
        proxy: None,
        accept_invalid_certs: true,
        timeout_secs: PROXY_PROBE_TIMEOUT_SECS,
    };
    let response = send_get(&candidate).await?;
    let _ = response.bytes().await.map_err(|e| e.to_string())?;
    Ok(())
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

fn create_strategy_client(candidate: &RequestCandidate) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(candidate.timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(candidate.accept_invalid_certs);

    if let Some(proxy) = &candidate.proxy {
        let proxy_url = proxy.to_url();
        let reqwest_proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| format!("创建代理失败 {}: {}", proxy_url, e))?;
        builder = builder.proxy(reqwest_proxy);
    }

    builder.build().map_err(|e| format!("构建 HTTP 客户端失败: {}", e))
}

async fn stream_response_to_file(
    response: reqwest::Response,
    target_path: &Path,
    on_progress: &mut (impl FnMut(GitHubDownloadProgress) + Send),
) -> Result<(), String> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建下载目录失败 {}: {}", parent.display(), e))?;
    }

    let mut file = fs::File::create(target_path)
        .map_err(|e| format!("创建下载文件失败 {}: {}", target_path.display(), e))?;
    let content_length = response.content_length();
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();

    // 中文说明：每个候选路由开始下载时先发送 0% 进度，让前端可感知重试切换。
    on_progress(GitHubDownloadProgress {
        chunk_length: 0,
        content_length,
        downloaded,
        percentage: 0.0,
    });

    while let Some(chunk) = stream.next().await {
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
    serde_json::from_str(&raw).ok()
}

fn write_proxy_cache(cache: &GitHubProxyCache) -> Result<(), String> {
    let path = proxy_cache_path()
        .ok_or_else(|| "无法确定 GitHub 代理缓存路径".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建 GitHub 代理缓存目录失败: {}", e))?;
    }
    let raw = serde_json::to_string_pretty(cache)
        .map_err(|e| format!("序列化 GitHub 代理缓存失败: {}", e))?;
    fs::write(&path, raw)
        .map_err(|e| format!("写入 GitHub 代理缓存失败 {}: {}", path.display(), e))
}

fn proxy_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("sanshu").join(CACHE_FILE_NAME))
}
