// 网络相关模块
// 包含地理位置检测、代理检测和HTTP客户端构建功能

pub mod client;
pub mod commands;
pub mod geo;
pub mod github_strategy;
pub mod proxy;

pub use client::{create_download_client, create_http_client, create_update_client};
pub use geo::detect_geo_location;
pub use github_strategy::{
    download_verified_with_strategy_with_progress,
    download_verified_with_strategy_with_progress_and_cancel, download_with_strategy,
    download_with_strategy_with_progress, fetch_announcement_with_strategy,
    fetch_latest_release_with_strategy, refresh_github_proxy_cache,
};
pub use proxy::{ProxyDetector, ProxyInfo};
