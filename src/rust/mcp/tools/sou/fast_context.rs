use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::future::join_all;
use globset::GlobBuilder;
use regex::Regex;
use reqwest::Client;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

const API_BASE: &str = "https://server.self-serve.windsurf.com/exa.api_server_pb.ApiServerService";
const AUTH_BASE: &str = "https://server.self-serve.windsurf.com/exa.auth_pb.AuthService";
const WS_APP: &str = "windsurf";
const DEFAULT_WS_APP_VER: &str = "1.48.2";
const DEFAULT_WS_LS_VER: &str = "1.9544.35";
const DEFAULT_WS_MODEL: &str = "MODEL_SWE_1_6_FAST";
const MAX_TREE_BYTES: usize = 250 * 1024;
const RESULT_MAX_LINES: usize = 50;
const LINE_MAX_CHARS: usize = 250;
const FINAL_FORCE_ANSWER: &str =
    "You have no turns left. Now you MUST provide your final ANSWER, even if it's not complete.";

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub project_root: PathBuf,
    pub api_key: Option<String>,
    pub tree_depth: u8,
    pub max_turns: u8,
    pub max_results: u8,
    pub max_commands: u8,
    pub timeout_ms: u64,
    pub exclude_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub files: Vec<FastContextFile>,
    pub rg_patterns: Vec<String>,
    /// ToolExecutor 在 readfile 命令中读取过的文件内容缓存（key 为规范化绝对路径）
    /// 用于格式化层复用，避免重复 IO。
    pub file_cache: HashMap<String, String>,
    pub meta: Value,
}

#[derive(Debug, Clone)]
pub struct FastContextFile {
    pub path: Option<String>,
    pub full_path: Option<String>,
    pub ranges: Vec<[usize; 2]>,
}

#[derive(Debug, Clone)]
pub struct ApiKeyDetection {
    pub api_key: String,
    pub source: ApiKeySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeySource {
    Config,
    Env,
    WindsurfDb,
}

impl ApiKeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Env => "env",
            Self::WindsurfDb => "windsurf_db",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Config => "已保存配置",
            Self::Env => "WINDSURF_API_KEY 环境变量",
            Self::WindsurfDb => "Windsurf 本地登录库",
        }
    }
}

#[derive(Debug, Clone)]
struct RepoMap {
    tree: String,
    depth: u8,
    size_bytes: usize,
    fell_back: bool,
}

#[derive(Debug, Clone)]
struct ChatMessage {
    role: u64,
    content: String,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    tool_args_json: Option<String>,
    ref_call_id: Option<String>,
}

#[derive(Debug)]
struct ParsedToolCall {
    thinking: String,
    name: String,
    args: Value,
}

#[derive(Debug, Clone)]
struct FastContextError {
    code: String,
    message: String,
    status: Option<u16>,
}

impl FastContextError {
    fn status(status: reqwest::StatusCode) -> Self {
        let code = match status.as_u16() {
            413 => "PAYLOAD_TOO_LARGE",
            429 => "RATE_LIMITED",
            401 | 403 => "AUTH_ERROR",
            _ => "SERVER_ERROR",
        };
        Self {
            code: code.to_string(),
            message: format!("HTTP {}", status.as_u16()),
            status: Some(status.as_u16()),
        }
    }

    fn timeout(message: impl Into<String>) -> Self {
        Self {
            code: "TIMEOUT".to_string(),
            message: message.into(),
            status: None,
        }
    }

    fn network(message: impl Into<String>) -> Self {
        Self {
            code: "NETWORK_ERROR".to_string(),
            message: message.into(),
            status: None,
        }
    }
}

impl fmt::Display for FastContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FastContextError {}

type FcResult<T> = std::result::Result<T, FastContextError>;

/// 使用 Rust 原生实现执行 fast-context 检索，避免依赖 demo 目录中的 Node bridge。
pub async fn search(opts: SearchOptions) -> Result<SearchResult> {
    let started_at = Instant::now();
    log::info!(
        "[fast-context] 开始搜索: project_root={}, query_len={}, tree_depth={}, max_turns={}, max_results={}, max_commands={}, timeout_ms={}, exclude_count={}",
        opts.project_root.display(),
        opts.query.chars().count(),
        opts.tree_depth,
        opts.max_turns,
        opts.max_results,
        opts.max_commands,
        opts.timeout_ms,
        opts.exclude_paths.len()
    );

    let project_root = opts
        .project_root
        .canonicalize()
        .with_context(|| format!("无法解析项目路径: {}", opts.project_root.display()))?;
    if !project_root.is_dir() {
        return Err(anyhow!("项目路径不是目录: {}", project_root.display()));
    }
    log::info!(
        "[fast-context] 项目路径已解析: {}",
        project_root.display()
    );

    let detected_key = detect_api_key(opts.api_key.as_deref())
        .context("未找到 Windsurf API Key，请在设置中填写或登录 Windsurf 后重试")?;
    log::info!(
        "[fast-context] Windsurf API Key 已解析: source={}, label={}, masked={}, length={}",
        detected_key.source.as_str(),
        detected_key.source.label(),
        mask_api_key(&detected_key.api_key),
        detected_key.api_key.chars().count()
    );
    let api_key = detected_key.api_key;

    let client = Client::builder()
        .timeout(Duration::from_millis(opts.timeout_ms + 5000))
        .build()
        .context("创建 fast-context HTTP 客户端失败")?;

    log::info!("[fast-context] 开始获取 Windsurf JWT");
    let jwt = fetch_jwt(&client, &api_key).await?;
    log::info!("[fast-context] Windsurf JWT 获取成功: length={}", jwt.len());
    log::info!("[fast-context] 开始检查 Windsurf 限流状态");
    if !check_rate_limit(&client, &api_key, &jwt).await? {
        log::warn!("[fast-context] Windsurf 限流检查未通过");
        return Err(anyhow!("RATE_LIMITED: Windsurf 当前限流，请稍后重试"));
    }
    log::info!("[fast-context] Windsurf 限流检查通过");

    let repo_map = get_repo_map(&project_root, opts.tree_depth, &opts.exclude_paths);
    log::info!(
        "[fast-context] repo map 已生成: depth={}, size_bytes={}, fell_back={}",
        repo_map.depth,
        repo_map.size_bytes,
        repo_map.fell_back
    );
    // #2 Repo Map 智能化：附加 README / manifest 摘要，提升 LLM 首轮命中率
    let project_summary = build_project_summary(&project_root);
    // 并发版 ToolExecutor：Arc<Mutex<状态>> 让多条 restricted_exec 命令可并行
    let executor = Arc::new(ToolExecutor::new(project_root.clone()));
    let tool_defs = build_tool_definitions(opts.max_commands);
    let system_prompt = build_system_prompt(opts.max_turns, opts.max_commands, opts.max_results);
    let user_content = format!(
        "Problem Statement: {}\n\nRepo Map (tree -L {} /codebase):\n```text\n{}\n```{}",
        opts.query, repo_map.depth, repo_map.tree, project_summary
    );

    let mut messages = vec![
        ChatMessage::new(5, system_prompt),
        ChatMessage::new(1, user_content),
    ];
    let total_api_calls = opts.max_turns as usize + 1;
    let mut compensated_turns = 0usize;
    let mut force_answer_injected = false;

    let mut turn = 0usize;
    while turn < total_api_calls + compensated_turns {
        log::info!(
            "[fast-context] 搜索轮次开始: turn={}, messages={}, compensated_turns={}, force_answer_injected={}",
            turn + 1,
            messages.len(),
            compensated_turns,
            force_answer_injected
        );
        let proto = build_request(&api_key, &jwt, &messages, &tool_defs)?;
        let response = match streaming_request(&client, &proto, opts.timeout_ms, 2).await {
            Ok(response) => response,
            Err(err)
                if matches!(err.code.as_str(), "PAYLOAD_TOO_LARGE" | "TIMEOUT")
                    && messages.len() > 4 =>
            {
                log::warn!(
                    "[fast-context] 流式请求失败并触发上下文裁剪: code={}, status={:?}, messages={}",
                    err.code,
                    err.status,
                    messages.len()
                );
                trim_messages(&mut messages);
                let retry_proto = build_request(&api_key, &jwt, &messages, &tool_defs)?;
                streaming_request(&client, &retry_proto, opts.timeout_ms, 0)
                    .await
                    .map_err(|retry_err| anyhow!("{} (context_trimmed=true)", retry_err))?
            }
            Err(err) => return Err(anyhow!(err)),
        };
        log::debug!(
            "[fast-context] 搜索轮次响应已收到: turn={}, bytes={}",
            turn + 1,
            response.len()
        );

        let Some(tool_call) = parse_response(&response)? else {
            let text = parse_plain_response(&response);
            if text.trim().is_empty() {
                return Err(anyhow!("fast-context 未返回可解析响应"));
            }
            if text.starts_with("[Error]") {
                return Err(anyhow!(text));
            }
            log::warn!(
                "[fast-context] 未解析到工具调用，返回 plain response: length={}",
                text.len()
            );
            return Ok(SearchResult {
                files: Vec::new(),
                rg_patterns: executor.collected_rg_patterns().await,
                file_cache: executor.snapshot_read_cache().await,
                meta: build_meta(&repo_map, true, Some(text)),
            });
        };

        match tool_call.name.as_str() {
            "answer" => {
                let answer_xml = tool_call
                    .args
                    .get("answer")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let files = parse_answer(answer_xml, &project_root)?;
                log::info!(
                    "[fast-context] answer 解析完成: files={}, elapsed_ms={}",
                    files.len(),
                    started_at.elapsed().as_millis()
                );
                // #3 把 ToolExecutor 已缓存的 readfile 内容透传给格式化层，避免再读一次磁盘
                return Ok(SearchResult {
                    files,
                    rg_patterns: executor.collected_rg_patterns().await,
                    file_cache: executor.snapshot_read_cache().await,
                    meta: build_meta(&repo_map, true, None),
                });
            }
            "restricted_exec" => {
                let call_id = Uuid::new_v4().to_string();
                let args_json = serde_json::to_string(&tool_call.args)
                    .context("序列化 fast-context 工具参数失败")?;
                let valid_commands = count_valid_commands(&tool_call.args);
                // #5 检测重复命令（指纹与上一次相同），帮助诊断 LLM 浪费
                let dup_count = executor.count_repeat_commands(&tool_call.args).await;
                if dup_count > 0 {
                    log::warn!(
                        "[fast-context] 检测到重复命令: turn={}, dup_count={}",
                        turn + 1,
                        dup_count
                    );
                }
                log::info!(
                    "[fast-context] restricted_exec 调用: turn={}, valid_commands={}, dup_count={}",
                    turn + 1,
                    valid_commands,
                    dup_count
                );
                let results = ToolExecutor::exec_tool_call(executor.clone(), &tool_call.args).await;
                log::debug!(
                    "[fast-context] restricted_exec 返回: turn={}, output_len={}",
                    turn + 1,
                    results.len()
                );

                if valid_commands == 0 && compensated_turns < 2 {
                    compensated_turns += 1;
                }

                messages.push(ChatMessage {
                    role: 2,
                    content: tool_call.thinking,
                    tool_call_id: Some(call_id.clone()),
                    tool_name: Some("restricted_exec".to_string()),
                    tool_args_json: Some(args_json),
                    ref_call_id: None,
                });
                messages.push(ChatMessage {
                    role: 4,
                    content: results,
                    tool_call_id: None,
                    tool_name: None,
                    tool_args_json: None,
                    ref_call_id: Some(call_id),
                });

                let effective_turn = turn.saturating_sub(compensated_turns);
                if effective_turn >= opts.max_turns.saturating_sub(1) as usize
                    && !force_answer_injected
                {
                    log::info!(
                        "[fast-context] 搜索轮次即将耗尽，已注入强制 answer 提示: turn={}",
                        turn + 1
                    );
                    messages.push(ChatMessage::new(1, FINAL_FORCE_ANSWER.to_string()));
                    force_answer_injected = true;
                }
            }
            other => return Err(anyhow!("fast-context 返回未知工具调用: {}", other)),
        }
        turn += 1;
    }

    log::warn!(
        "[fast-context] 已达到最大轮次但未获得 answer: elapsed_ms={}",
        started_at.elapsed().as_millis()
    );
    Ok(SearchResult {
        files: Vec::new(),
        rg_patterns: executor.collected_rg_patterns().await,
        file_cache: executor.snapshot_read_cache().await,
        meta: build_meta(
            &repo_map,
            true,
            Some("Max turns reached without getting an answer".to_string()),
        ),
    })
}

impl ChatMessage {
    fn new(role: u64, content: String) -> Self {
        Self {
            role,
            content,
            tool_call_id: None,
            tool_name: None,
            tool_args_json: None,
            ref_call_id: None,
        }
    }
}

pub fn detect_api_key(configured: Option<&str>) -> Result<ApiKeyDetection> {
    if let Some(key) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(ApiKeyDetection {
            api_key: key.to_string(),
            source: ApiKeySource::Config,
        });
    }
    if let Ok(key) = env::var("WINDSURF_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Ok(ApiKeyDetection {
                api_key: key,
                source: ApiKeySource::Env,
            });
        }
    }
    extract_windsurf_api_key()?
        .map(|api_key| ApiKeyDetection {
            api_key,
            source: ApiKeySource::WindsurfDb,
        })
        .ok_or_else(|| anyhow!("Windsurf 本地登录库中没有 apiKey"))
}

pub fn mask_api_key(api_key: &str) -> String {
    let trimmed = api_key.trim();
    let char_count = trimmed.chars().count();
    if char_count <= 8 {
        return "*".repeat(char_count.max(1));
    }
    let prefix = trimmed.chars().take(4).collect::<String>();
    let suffix = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}...{suffix}")
}

fn extract_windsurf_api_key() -> Result<Option<String>> {
    let db_path = windsurf_state_db_path()?;
    if !db_path.exists() {
        log::warn!(
            "[fast-context] Windsurf 登录数据库不存在: {}",
            db_path.display()
        );
        return Ok(None);
    }
    log::debug!(
        "[fast-context] 尝试读取 Windsurf 登录数据库: {}",
        db_path.display()
    );

    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("打开 Windsurf 登录数据库失败: {}", db_path.display()))?;
    let value: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'windsurfAuthStatus'",
            [],
            |row| row.get(0),
        )
        .context("读取 windsurfAuthStatus 记录失败")?;
    let json: Value = serde_json::from_str(&value).context("解析 windsurfAuthStatus JSON 失败")?;
    Ok(json
        .get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned))
}

fn windsurf_state_db_path() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("无法定位用户主目录"))?;
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("Windsurf")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"));
    }
    if cfg!(target_os = "windows") {
        let appdata = env::var("APPDATA").context("无法读取 APPDATA 环境变量")?;
        return Ok(PathBuf::from(appdata)
            .join("Windsurf")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"));
    }

    let config_root = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            dirs::home_dir()
                .map(|home| home.join(".config"))
                .ok_or(env::VarError::NotPresent)
        })
        .context("无法定位 Linux 配置目录")?;
    Ok(config_root
        .join("Windsurf")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

async fn fetch_jwt(client: &Client, api_key: &str) -> Result<String> {
    let mut meta = ProtobufEncoder::default();
    meta.write_string(1, WS_APP);
    meta.write_string(2, &ws_app_ver());
    meta.write_string(3, api_key);
    meta.write_string(4, "zh-cn");
    meta.write_string(7, &ws_ls_ver());
    meta.write_string(12, WS_APP);
    meta.write_bytes(30, &[0x00, 0x01]);

    let mut outer = ProtobufEncoder::default();
    outer.write_message(1, &meta);
    let response = unary_request(
        client,
        &format!("{AUTH_BASE}/GetUserJwt"),
        &outer.into_bytes(),
        false,
        30_000,
    )
    .await
    .map_err(|e| anyhow!("获取 Windsurf JWT 失败: {}", e))?;

    extract_jwt_from_response(&response).ok_or_else(|| anyhow!("无法从 GetUserJwt 响应中提取 JWT"))
}

fn extract_jwt_from_response(response: &[u8]) -> Option<String> {
    let mut candidates = vec![response.to_vec()];

    if let Ok(decoded) = gunzip_bytes(response) {
        candidates.push(decoded);
    }
    candidates.extend(connect_frame_decode(response));

    for bytes in candidates {
        for value in extract_strings(&bytes) {
            let trimmed = value.trim();
            if trimmed.starts_with("eyJ") && trimmed.contains('.') {
                return Some(trimmed.to_string());
            }
        }

        let raw_text = String::from_utf8_lossy(&bytes);
        for part in raw_text.split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ','))
        {
            let trimmed = part.trim();
            if trimmed.starts_with("eyJ") && trimmed.contains('.') {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

async fn check_rate_limit(client: &Client, api_key: &str, jwt: &str) -> Result<bool> {
    let mut request = ProtobufEncoder::default();
    request.write_message(1, &build_metadata(api_key, jwt)?);
    request.write_string(3, &ws_model());

    match unary_request(
        client,
        &format!("{API_BASE}/CheckUserMessageRateLimit"),
        &request.into_bytes(),
        true,
        30_000,
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(err) if err.status == Some(429) => Ok(false),
        // #4 严格化：非 429 网络/HTTP 错误向上抛，避免后续浪费 LLM 配额
        Err(err) => Err(anyhow!("rate-limit 检查失败: {}", err)),
    }
}

async fn unary_request(
    client: &Client,
    url: &str,
    proto_bytes: &[u8],
    compress: bool,
    timeout_ms: u64,
) -> FcResult<Vec<u8>> {
    let started_at = Instant::now();
    let mut body = proto_bytes.to_vec();
    let mut request = client
        .post(url)
        .timeout(Duration::from_millis(timeout_ms))
        .header("Content-Type", "application/proto")
        .header("Connect-Protocol-Version", "1")
        .header("User-Agent", "connect-go/1.18.1 (go1.25.5)")
        .header("Accept-Encoding", "gzip");

    if compress {
        body = gzip_bytes(proto_bytes)?;
        request = request.header("Content-Encoding", "gzip");
    }

    let response = request
        .body(body)
        .send()
        .await
        .map_err(classify_reqwest_error)?;
    let status = response.status();
    if !status.is_success() {
        log::warn!(
            "[fast-context] unary 请求失败: url={}, status={}, elapsed_ms={}",
            url,
            status,
            started_at.elapsed().as_millis()
        );
        return Err(FastContextError::status(status));
    }
    let bytes = response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(classify_reqwest_error)?;
    log::info!(
        "[fast-context] unary 请求成功: url={}, bytes={}, elapsed_ms={}",
        url,
        bytes.len(),
        started_at.elapsed().as_millis()
    );
    Ok(bytes)
}

async fn streaming_request(
    client: &Client,
    proto_bytes: &[u8],
    timeout_ms: u64,
    max_retries: usize,
) -> FcResult<Vec<u8>> {
    let frame = connect_frame_encode(proto_bytes, true)?;
    let url = format!("{API_BASE}/GetDevstralStream");
    let base_timeout_ms = timeout_ms.max(1000);
    let abort_ms = base_timeout_ms + 5000;
    let mut last_error = None;

    for attempt in 0..=max_retries {
        let started_at = Instant::now();
        let trace_id = Uuid::new_v4().simple().to_string();
        let span_id = Uuid::new_v4().simple().to_string()[..16].to_string();
        let response = client
            .post(&url)
            .timeout(Duration::from_millis(abort_ms))
            .header("Content-Type", "application/connect+proto")
            .header("Connect-Protocol-Version", "1")
            .header("Connect-Accept-Encoding", "gzip")
            .header("Connect-Content-Encoding", "gzip")
            .header("Connect-Timeout-Ms", base_timeout_ms.to_string())
            .header("User-Agent", "connect-go/1.18.1 (go1.25.5)")
            .header("Accept-Encoding", "identity")
            .header(
                "Baggage",
                format!(
                    "sentry-release=language-server-windsurf@{},sentry-environment=stable,sentry-sampled=false,sentry-trace_id={},sentry-public_key=b813f73488da69eedec534dba1029111",
                    ws_ls_ver(), trace_id
                ),
            )
            .header("Sentry-Trace", format!("{}-{}-0", trace_id, span_id))
            .body(frame.clone())
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp
                    .bytes()
                    .await
                    .map(|bytes| bytes.to_vec())
                    .map_err(classify_reqwest_error)?;
                log::info!(
                    "[fast-context] 流式请求成功: attempt={}, bytes={}, elapsed_ms={}",
                    attempt + 1,
                    bytes.len(),
                    started_at.elapsed().as_millis()
                );
                return Ok(bytes);
            }
            Ok(resp) => {
                let err = FastContextError::status(resp.status());
                log::warn!(
                    "[fast-context] 流式请求 HTTP 失败: attempt={}, status={:?}, code={}, elapsed_ms={}",
                    attempt + 1,
                    err.status,
                    err.code,
                    started_at.elapsed().as_millis()
                );
                // #4 429 速率限制不应重试（继续打只会更糟），4xx（除外）也不重试
                if err.status == Some(429) {
                    return Err(err);
                }
                if err
                    .status
                    .is_some_and(|s| (400..500).contains(&s) && s != 429)
                {
                    return Err(err);
                }
                last_error = Some(err);
            }
            Err(err) => {
                let err = classify_reqwest_error(err);
                log::warn!(
                    "[fast-context] 流式请求网络失败: attempt={}, code={}, elapsed_ms={}, message={}",
                    attempt + 1,
                    err.code,
                    started_at.elapsed().as_millis(),
                    err.message
                );
                last_error = Some(err);
            }
        }

        if attempt < max_retries {
            // #4 指数退避 + jitter：避免雷霆群与服务器同步震荡
            let base_ms = 1000u64 * (attempt as u64 + 1);
            let jitter_ms = pseudo_jitter_ms(attempt);
            tokio::time::sleep(Duration::from_millis(base_ms + jitter_ms)).await;
        }
    }

    Err(last_error.unwrap_or_else(|| FastContextError::timeout("streaming request timed out")))
}

/// 基于纳秒时间戳的轻量 jitter（0~400ms），无需引入 rand 依赖
fn pseudo_jitter_ms(attempt: usize) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let seed = nanos.wrapping_add(attempt as u64 * 7919);
    seed % 400
}

fn classify_reqwest_error(err: reqwest::Error) -> FastContextError {
    if err.is_timeout() {
        FastContextError::timeout(err.to_string())
    } else if let Some(status) = err.status() {
        FastContextError::status(status)
    } else {
        FastContextError::network(err.to_string())
    }
}

fn build_metadata(api_key: &str, jwt: &str) -> Result<ProtobufEncoder> {
    let mut meta = ProtobufEncoder::default();
    meta.write_string(1, WS_APP);
    meta.write_string(2, &ws_app_ver());
    meta.write_string(3, api_key);
    meta.write_string(4, "zh-cn");
    meta.write_string(5, &serde_json::to_string(&system_info())?);
    meta.write_string(7, &ws_ls_ver());
    meta.write_string(8, &serde_json::to_string(&cpu_info())?);
    meta.write_string(12, WS_APP);
    meta.write_string(21, jwt);
    meta.write_bytes(30, &[0x00, 0x01]);
    Ok(meta)
}

fn build_request(
    api_key: &str,
    jwt: &str,
    messages: &[ChatMessage],
    tool_defs: &str,
) -> Result<Vec<u8>> {
    let mut request = ProtobufEncoder::default();
    request.write_message(1, &build_metadata(api_key, jwt)?);
    for message in messages {
        request.write_message(2, &build_chat_message(message));
    }
    request.write_string(3, tool_defs);
    Ok(request.into_bytes())
}

fn build_chat_message(message: &ChatMessage) -> ProtobufEncoder {
    let mut msg = ProtobufEncoder::default();
    msg.write_varint(2, message.role);
    msg.write_string(3, &message.content);

    if let (Some(call_id), Some(tool_name), Some(args_json)) = (
        message.tool_call_id.as_deref(),
        message.tool_name.as_deref(),
        message.tool_args_json.as_deref(),
    ) {
        let mut tool_call = ProtobufEncoder::default();
        tool_call.write_string(1, call_id);
        tool_call.write_string(2, tool_name);
        tool_call.write_string(3, args_json);
        msg.write_message(6, &tool_call);
    }

    if let Some(ref_call_id) = message.ref_call_id.as_deref() {
        msg.write_string(7, ref_call_id);
    }

    msg
}

fn parse_response(data: &[u8]) -> Result<Option<ParsedToolCall>> {
    let mut all_text = String::new();
    for frame in connect_frame_decode(data) {
        let text_candidate = String::from_utf8_lossy(&frame);
        if text_candidate.starts_with('{') {
            if let Ok(err_obj) = serde_json::from_str::<Value>(&text_candidate) {
                if let Some(error) = err_obj.get("error") {
                    let code = error
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
                    return Err(anyhow!("[Error] {code}: {message}"));
                }
            }
        }

        let raw_text = text_candidate.replace('\u{fffd}', "");
        if raw_text.contains("[TOOL_CALLS]") {
            all_text = raw_text;
            break;
        }

        for s in extract_strings(&frame) {
            if s.len() > 10 {
                all_text.push_str(&s);
            }
        }
    }

    Ok(parse_tool_call(&all_text))
}

fn parse_plain_response(data: &[u8]) -> String {
    connect_frame_decode(data)
        .into_iter()
        .flat_map(|frame| extract_strings(&frame))
        .filter(|s| s.len() > 10)
        .collect::<Vec<_>>()
        .join("")
}

fn parse_tool_call(text: &str) -> Option<ParsedToolCall> {
    let text = text.replace("</s>", "");
    let marker = "[TOOL_CALLS]";
    let args_marker = "[ARGS]";
    let marker_start = text.find(marker)?;
    let name_start = marker_start + marker.len();
    let args_start_rel = text[name_start..].find(args_marker)?;
    let name = text[name_start..name_start + args_start_rel].trim();
    if name.is_empty() {
        return None;
    }

    let raw = text[name_start + args_start_rel + args_marker.len()..].trim();
    let end = find_json_object_end(raw).unwrap_or(raw.len());
    let args = serde_json::from_str(&raw[..end]).ok()?;
    Some(ParsedToolCall {
        thinking: text[..marker_start].trim().to_string(),
        name: name.to_string(),
        args,
    })
}

fn find_json_object_end(raw: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_answer(xml_text: &str, project_root: &Path) -> Result<Vec<FastContextFile>> {
    let file_re =
        Regex::new(r#"(?s)<file\s+path=["']([^"']+)["']>(.*?)</file>"#).expect("valid regex");
    let range_re = Regex::new(r"<range>(\d+)-(\d+)</range>").expect("valid regex");
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut files = Vec::new();

    for cap in file_re.captures_iter(xml_text) {
        let vpath = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let body = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        let Some((rel_path, full_path)) = resolve_answer_path(vpath, &root) else {
            continue;
        };

        let ranges = range_re
            .captures_iter(body)
            .filter_map(|range_cap| {
                let start = range_cap.get(1)?.as_str().parse::<usize>().ok()?;
                let end = range_cap.get(2)?.as_str().parse::<usize>().ok()?;
                Some([start.max(1), end.max(start)])
            })
            .collect::<Vec<_>>();

        files.push(FastContextFile {
            path: Some(rel_path),
            full_path: Some(normalize_path(&full_path)),
            ranges,
        });
    }

    Ok(files)
}

fn resolve_answer_path(vpath: &str, root: &Path) -> Option<(String, PathBuf)> {
    let mut normalized = vpath.trim().replace('\\', "/");
    if normalized.starts_with("/codebase") {
        normalized = normalized
            .trim_start_matches("/codebase")
            .trim_start_matches('/')
            .to_string();
    }

    let candidate = if Path::new(&normalized).is_absolute() {
        PathBuf::from(&normalized)
    } else {
        if has_parent_dir(Path::new(&normalized)) {
            return None;
        }
        root.join(&normalized)
    };
    let absolute = candidate.canonicalize().unwrap_or(candidate);
    if !absolute.starts_with(root) {
        return None;
    }
    let rel = absolute.strip_prefix(root).ok().map(normalize_path)?;
    Some((rel, absolute))
}

fn get_repo_map(project_root: &Path, target_depth: u8, exclude_paths: &[String]) -> RepoMap {
    for depth in (1..=target_depth.max(1)).rev() {
        let tree = build_tree(project_root, "/codebase", depth, exclude_paths);
        let size_bytes = tree.len();
        if size_bytes <= MAX_TREE_BYTES {
            return RepoMap {
                tree,
                depth,
                size_bytes,
                fell_back: depth < target_depth,
            };
        }
    }

    let tree = list_root(project_root, exclude_paths);
    RepoMap {
        size_bytes: tree.len(),
        tree,
        depth: 0,
        fell_back: true,
    }
}

fn build_tree(root: &Path, label: &str, max_depth: u8, exclude_paths: &[String]) -> String {
    let mut lines = vec![label.to_string()];
    append_tree(root, "", 1, max_depth, exclude_paths, &mut lines);
    lines.join("\n")
}

fn append_tree(
    dir: &Path,
    prefix: &str,
    depth: u8,
    max_depth: u8,
    exclude_paths: &[String],
    lines: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
    }

    let entries = sorted_entries(dir, exclude_paths);
    let len = entries.len();
    for (idx, entry) in entries.into_iter().enumerate() {
        let name = entry.file_name().to_string_lossy().to_string();
        let last = idx + 1 == len;
        // #2 给目录追加 "(N entries)" 后缀，避免 LLM 盲探巨型目录
        let display_name = if entry.path().is_dir() {
            let count = entry_count(&entry.path(), exclude_paths);
            if count > 0 {
                format!("{name}/ ({count} entries)")
            } else {
                format!("{name}/")
            }
        } else {
            name
        };
        lines.push(format!(
            "{}{} {}",
            prefix,
            if last { "`--" } else { "|--" },
            display_name
        ));
        if entry.path().is_dir() {
            let next_prefix = format!("{}{}", prefix, if last { "   " } else { "|  " });
            append_tree(
                &entry.path(),
                &next_prefix,
                depth + 1,
                max_depth,
                exclude_paths,
                lines,
            );
        }
    }
}

/// 浅层统计目录下条目数（不递归），用于 tree 显示规模线索
fn entry_count(dir: &Path, exclude_paths: &[String]) -> usize {
    fs::read_dir(dir)
        .map(|read_dir| {
            read_dir
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    !matches_exclude(&name, exclude_paths)
                })
                .count()
        })
        .unwrap_or(0)
}

/// #2 构建项目简介：README 首段 + manifest 顶层信息
/// 目的：让 LLM 第一轮就知道项目用什么技术栈、入口在哪，省一轮 ls/tree 探查
fn build_project_summary(root: &Path) -> String {
    let mut sections = Vec::new();

    // README 首 30 行（截断）
    for candidate in ["README.md", "README.MD", "Readme.md", "readme.md", "README"] {
        let path = root.join(candidate);
        if let Ok(content) = fs::read_to_string(&path) {
            let head = content
                .lines()
                .take(30)
                .collect::<Vec<_>>()
                .join("\n");
            if !head.trim().is_empty() {
                sections.push(format!(
                    "### README ({candidate}, first 30 lines)\n```\n{head}\n```"
                ));
                break;
            }
        }
    }

    // Cargo.toml workspace / package 顶层
    if let Ok(content) = fs::read_to_string(root.join("Cargo.toml")) {
        let head = content.lines().take(40).collect::<Vec<_>>().join("\n");
        if !head.trim().is_empty() {
            sections.push(format!("### Cargo.toml (first 40 lines)\n```toml\n{head}\n```"));
        }
    }

    // package.json 顶层
    if let Ok(content) = fs::read_to_string(root.join("package.json")) {
        let head = content.lines().take(40).collect::<Vec<_>>().join("\n");
        if !head.trim().is_empty() {
            sections.push(format!(
                "### package.json (first 40 lines)\n```json\n{head}\n```"
            ));
        }
    }

    // pyproject.toml
    if let Ok(content) = fs::read_to_string(root.join("pyproject.toml")) {
        let head = content.lines().take(30).collect::<Vec<_>>().join("\n");
        if !head.trim().is_empty() {
            sections.push(format!(
                "### pyproject.toml (first 30 lines)\n```toml\n{head}\n```"
            ));
        }
    }

    if sections.is_empty() {
        String::new()
    } else {
        format!("\n\nProject Summary (auto-extracted):\n{}", sections.join("\n\n"))
    }
}

fn list_root(root: &Path, exclude_paths: &[String]) -> String {
    let mut lines = vec!["/codebase".to_string()];
    for entry in sorted_entries(root, exclude_paths) {
        lines.push(format!("|-- {}", entry.file_name().to_string_lossy()));
    }
    lines.join("\n")
}

fn sorted_entries(dir: &Path, exclude_paths: &[String]) -> Vec<fs::DirEntry> {
    let mut entries = match fs::read_dir(dir) {
        Ok(read_dir) => read_dir.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    entries.retain(|entry| {
        let name = entry.file_name().to_string_lossy().to_string();
        !matches_exclude(&name, exclude_paths)
    });
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
    entries
}

fn matches_exclude(name: &str, exclude_paths: &[String]) -> bool {
    exclude_paths.iter().any(|pattern| {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return false;
        }
        if !pattern.contains('*') && !pattern.contains('?') {
            return pattern == name;
        }
        glob_match(pattern, name)
    })
}

fn glob_match(pattern: &str, text: &str) -> bool {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher().is_match(text))
        .unwrap_or(false)
}

fn build_system_prompt(max_turns: u8, max_commands: u8, max_results: u8) -> String {
    format!(
        r#"You are an expert software engineer providing code context for another engineer.
Return only the files and inclusive line ranges needed to understand and implement the user's request.

Environment:
- Working directory is /codebase.
- Tool-call protocol is text based: call tools by outputting `[TOOL_CALLS]restricted_exec[ARGS]{{...}}` or `[TOOL_CALLS]answer[ARGS]{{...}}` exactly.
- You may use exactly one restricted_exec tool call per search turn.
- Each restricted_exec call may include at most {max_commands} commands.
- Available command types: rg, readfile, tree, ls, glob.
- Prefer narrow rg searches first, then read complete semantic blocks.
- Avoid generated, vendored, dependency, build, and cache directories unless directly relevant.
- You have at most {max_turns} search turns before final answer.

Final answer:
- Use the answer tool by outputting `[TOOL_CALLS]answer[ARGS]{{"answer":"<ANSWER>...</ANSWER>"}}`.
- answer must be XML with root <ANSWER>.
- Use <file path="/codebase/path"><range>start-end</range></file>.
- Aim for at most {max_results} files.
- If nothing relevant exists, return <ANSWER></ANSWER>.
"#
    )
}

fn build_tool_definitions(max_commands: u8) -> String {
    let mut props = Map::new();
    for i in 1..=max_commands.max(1) {
        props.insert(format!("command{i}"), command_schema(i));
    }

    json!([
        {
            "type": "function",
            "function": {
                "name": "restricted_exec",
                "description": "Execute restricted commands in parallel.",
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": ["command1"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "answer",
                "description": "Final answer with relevant files and line ranges.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "answer": { "type": "string", "description": "The final answer in XML format." }
                    },
                    "required": ["answer"]
                }
            }
        }
    ])
    .to_string()
}

fn command_schema(n: u8) -> Value {
    json!({
        "type": "object",
        "description": format!("Command {n} to execute."),
        "oneOf": [
            {
                "properties": {
                    "type": { "type": "string", "const": "rg" },
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "include": { "type": "array", "items": { "type": "string" } },
                    "exclude": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["type", "pattern", "path"]
            },
            {
                "properties": {
                    "type": { "type": "string", "const": "readfile" },
                    "file": { "type": "string" },
                    "start_line": { "type": "integer" },
                    "end_line": { "type": "integer" }
                },
                "required": ["type", "file"]
            },
            {
                "properties": {
                    "type": { "type": "string", "const": "tree" },
                    "path": { "type": "string" },
                    "levels": { "type": "integer" }
                },
                "required": ["type", "path"]
            },
            {
                "properties": {
                    "type": { "type": "string", "const": "ls" },
                    "path": { "type": "string" },
                    "long_format": { "type": "boolean" },
                    "all": { "type": "boolean" }
                },
                "required": ["type", "path"]
            },
            {
                "properties": {
                    "type": { "type": "string", "const": "glob" },
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "type_filter": { "type": "string", "enum": ["file", "directory", "all"] }
                },
                "required": ["type", "pattern", "path"]
            }
        ]
    })
}

fn count_valid_commands(args: &Value) -> usize {
    args.as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(key, value)| {
                    key.starts_with("command")
                        && value.get("type").and_then(Value::as_str).is_some()
                })
                .count()
        })
        .unwrap_or(0)
}

fn trim_messages(messages: &mut Vec<ChatMessage>) {
    if messages.len() <= 4 {
        return;
    }
    let mut trimmed = Vec::new();
    trimmed.extend_from_slice(&messages[..2]);
    trimmed.push(ChatMessage::new(
        1,
        "[Prior search rounds omitted to reduce payload. Provide your best answer based on available context.]".to_string(),
    ));
    trimmed.extend_from_slice(&messages[messages.len() - 2..]);
    *messages = trimmed;
}

fn build_meta(repo_map: &RepoMap, native: bool, raw_response: Option<String>) -> Value {
    let mut meta = json!({
        "treeDepth": repo_map.depth,
        "treeSizeKB": ((repo_map.size_bytes as f64 / 1024.0) * 10.0).round() / 10.0,
        "fellBack": repo_map.fell_back,
        "native": native
    });
    if let Some(raw) = raw_response {
        meta["raw_response"] = Value::String(raw);
    }
    meta
}

#[derive(Default)]
struct ProtobufEncoder {
    bytes: Vec<u8>,
}

impl ProtobufEncoder {
    fn write_varint(&mut self, field: u64, value: u64) {
        self.bytes.extend(encode_varint((field << 3) | 0));
        self.bytes.extend(encode_varint(value));
    }

    fn write_string(&mut self, field: u64, value: &str) {
        self.write_bytes(field, value.as_bytes());
    }

    fn write_bytes(&mut self, field: u64, value: &[u8]) {
        self.bytes.extend(encode_varint((field << 3) | 2));
        self.bytes.extend(encode_varint(value.len() as u64));
        self.bytes.extend(value);
    }

    fn write_message(&mut self, field: u64, sub: &ProtobufEncoder) {
        self.write_bytes(field, &sub.bytes);
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value > 0x7f {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push((value & 0x7f) as u8);
    out
}

fn decode_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    while *offset < data.len() {
        let b = data[*offset];
        *offset += 1;
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

fn extract_strings(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    extract_strings_inner(data, 0, &mut out);
    out
}

fn extract_strings_inner(data: &[u8], depth: u8, out: &mut Vec<String>) {
    if depth > 3 {
        return;
    }
    let mut i = 0usize;
    while i < data.len() {
        let Some(tag) = decode_varint(data, &mut i) else {
            break;
        };
        match tag & 0x7 {
            0 => {
                let _ = decode_varint(data, &mut i);
            }
            1 => i = i.saturating_add(8).min(data.len()),
            2 => {
                let Some(length) = decode_varint(data, &mut i).map(|v| v as usize) else {
                    break;
                };
                if i + length > data.len() {
                    break;
                }
                let raw = &data[i..i + length];
                let text = String::from_utf8_lossy(raw).replace('\u{fffd}', "");
                if text.len() > 5 && printable_score(&text) > 0.75 {
                    out.push(text);
                }
                extract_strings_inner(raw, depth + 1, out);
                i += length;
            }
            5 => i = i.saturating_add(4).min(data.len()),
            _ => break,
        }
    }
}

fn printable_score(text: &str) -> f32 {
    let total = text.chars().count().max(1) as f32;
    let printable = text
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
        .count() as f32;
    printable / total
}

fn connect_frame_encode(proto_bytes: &[u8], compress: bool) -> FcResult<Vec<u8>> {
    let (flags, payload) = if compress {
        (1u8, gzip_bytes(proto_bytes)?)
    } else {
        (0u8, proto_bytes.to_vec())
    };
    let mut frame = Vec::with_capacity(payload.len() + 5);
    frame.push(flags);
    frame.extend((payload.len() as u32).to_be_bytes());
    frame.extend(payload);
    Ok(frame)
}

fn connect_frame_decode(data: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut i = 0usize;
    while i + 5 <= data.len() {
        let flags = data[i];
        let length =
            u32::from_be_bytes([data[i + 1], data[i + 2], data[i + 3], data[i + 4]]) as usize;
        i += 5;
        if i + length > data.len() {
            break;
        }
        let payload = &data[i..i + length];
        i += length;
        if matches!(flags, 1 | 3) {
            frames.push(gunzip_bytes(payload).unwrap_or_else(|_| payload.to_vec()));
        } else {
            frames.push(payload.to_vec());
        }
    }
    frames
}

fn gzip_bytes(data: &[u8]) -> FcResult<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| FastContextError::network(e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| FastContextError::network(e.to_string()))
}

fn gunzip_bytes(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

struct ToolExecutor {
    root: PathBuf,
    root_slash: String,
    state: Mutex<ToolExecutorState>,
}

#[derive(Default)]
struct ToolExecutorState {
    /// rg pattern 收集（向外返回）
    collected_rg_patterns: Vec<String>,
    /// 命令指纹 → 输出缓存：跨 turn 命中可零成本返回，节省 LLM 重复探查的时间
    command_cache: HashMap<String, String>,
    /// readfile 完整文件缓存：(规范化绝对路径 → 文件原文)
    /// 供格式化层复用；同一文件的多个 readfile 也共享一次磁盘 IO
    read_cache: HashMap<String, String>,
    /// 上一次 turn 的命令指纹集合，用于 #5 重复检测
    last_turn_fingerprints: HashSet<String>,
}

impl ToolExecutor {
    fn new(root: PathBuf) -> Self {
        let root_slash = normalize_path(&root);
        Self {
            root,
            root_slash,
            state: Mutex::new(ToolExecutorState::default()),
        }
    }

    async fn collected_rg_patterns(&self) -> Vec<String> {
        let state = self.state.lock().await;
        let mut seen = HashSet::new();
        state
            .collected_rg_patterns
            .iter()
            .filter(|pattern| seen.insert((*pattern).clone()))
            .cloned()
            .collect()
    }

    /// #3 暴露 readfile 内容快照，供 format_fast_context_text 复用，避免重复读盘
    async fn snapshot_read_cache(&self) -> HashMap<String, String> {
        self.state.lock().await.read_cache.clone()
    }

    /// #5 统计当前 args 中与上一次相同的命令指纹数量
    async fn count_repeat_commands(&self, args: &Value) -> usize {
        let state = self.state.lock().await;
        let mut dup = 0usize;
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                if !key.starts_with("command") {
                    continue;
                }
                let fp = command_fingerprint(value);
                if !fp.is_empty() && state.last_turn_fingerprints.contains(&fp) {
                    dup += 1;
                }
            }
        }
        dup
    }

    /// 并发执行一次 restricted_exec 中的所有子命令，返回拼接结果。
    /// 接收 Arc<Self> 是为了在 join_all 里把同一个 executor 复制给多个并发 future。
    async fn exec_tool_call(self_arc: Arc<Self>, args: &Value) -> String {
        let Some(obj) = args.as_object() else {
            log::warn!("[fast-context] restricted_exec 参数缺失或格式错误");
            return "Error: missing or invalid tool args".to_string();
        };
        let mut keys = obj
            .keys()
            .filter(|key| key.starts_with("command"))
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();

        log::info!(
            "[fast-context] restricted_exec 开始并发执行本地命令: count={}",
            keys.len()
        );

        // 记录本轮指纹（覆盖上一轮），用于下一轮的重复检测
        let mut current_fps = HashSet::new();
        for key in &keys {
            let fp = command_fingerprint(&obj[key]);
            if !fp.is_empty() {
                current_fps.insert(fp);
            }
        }

        // 并发：每条命令一个 future
        let futures = keys.iter().map(|key| {
            let executor = self_arc.clone();
            let key_owned = key.clone();
            let cmd = obj[key].clone();
            async move {
                let started_at = Instant::now();
                let output = executor.exec_command(&cmd).await;
                log::info!(
                    "[fast-context] restricted_exec 本地命令完成: key={}, output_len={}, elapsed_ms={}",
                    key_owned,
                    output.len(),
                    started_at.elapsed().as_millis()
                );
                format!("<{key_owned}_result>\n{output}\n</{key_owned}_result>")
            }
        });
        let parts: Vec<String> = join_all(futures).await;

        // 更新最后一轮指纹（不影响当轮 dup_count，因为 dup_count 在 exec 之前已检测）
        {
            let mut state = self_arc.state.lock().await;
            state.last_turn_fingerprints = current_fps;
        }

        parts.join("")
    }

    async fn exec_command(&self, cmd: &Value) -> String {
        let Some(kind) = cmd.get("type").and_then(Value::as_str) else {
            log::warn!("[fast-context] 本地命令缺少 type: {}", cmd);
            return "Error: missing or invalid command".to_string();
        };

        // 命令缓存：相同指纹直接复用结果（跨 turn 都生效）
        let fp = command_fingerprint(cmd);
        if !fp.is_empty() {
            let state = self.state.lock().await;
            if let Some(cached) = state.command_cache.get(&fp) {
                log::info!(
                    "[fast-context] 命令缓存命中: kind={}, fp_len={}, output_len={}",
                    kind,
                    fp.len(),
                    cached.len()
                );
                return cached.clone();
            }
        }

        let output = match kind {
            "rg" => {
                let pattern = cmd.get("pattern").and_then(Value::as_str).unwrap_or("");
                let path = cmd.get("path").and_then(Value::as_str).unwrap_or("");
                let include = string_array(cmd.get("include"));
                let exclude = string_array(cmd.get("exclude"));
                log::info!(
                    "[fast-context] 本地命令 rg: path={}, pattern_len={}, include_count={}, exclude_count={}",
                    path,
                    pattern.chars().count(),
                    include.len(),
                    exclude.len()
                );
                self.rg(pattern, path, include, exclude).await
            }
            "readfile" => {
                let file = cmd.get("file").and_then(Value::as_str).unwrap_or("");
                let start = cmd
                    .get("start_line")
                    .and_then(Value::as_u64)
                    .map(|v| v as usize);
                let end = cmd
                    .get("end_line")
                    .and_then(Value::as_u64)
                    .map(|v| v as usize);
                log::info!(
                    "[fast-context] 本地命令 readfile: file={}, start_line={:?}, end_line={:?}",
                    file,
                    start,
                    end
                );
                self.readfile(file, start, end).await
            }
            "tree" => {
                let path = cmd.get("path").and_then(Value::as_str).unwrap_or("");
                let levels = cmd.get("levels").and_then(Value::as_u64).map(|v| v as u8);
                log::info!(
                    "[fast-context] 本地命令 tree: path={}, levels={:?}",
                    path,
                    levels
                );
                self.tree(path, levels)
            }
            "ls" => {
                let path = cmd.get("path").and_then(Value::as_str).unwrap_or("");
                let long_format = cmd
                    .get("long_format")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let all = cmd.get("all").and_then(Value::as_bool).unwrap_or(false);
                log::info!(
                    "[fast-context] 本地命令 ls: path={}, long_format={}, all={}",
                    path,
                    long_format,
                    all
                );
                self.ls(path, long_format, all)
            }
            "glob" => {
                let pattern = cmd.get("pattern").and_then(Value::as_str).unwrap_or("");
                let path = cmd.get("path").and_then(Value::as_str).unwrap_or("");
                let type_filter = cmd
                    .get("type_filter")
                    .and_then(Value::as_str)
                    .unwrap_or("all");
                log::info!(
                    "[fast-context] 本地命令 glob: path={}, pattern={}, type_filter={}",
                    path,
                    pattern,
                    type_filter
                );
                self.glob(pattern, path, type_filter)
            }
            other => {
                log::warn!("[fast-context] 未知本地命令类型: {}", other);
                format!("Error: unknown command type '{other}'")
            }
        };

        // 写入命令缓存
        if !fp.is_empty() {
            let mut state = self.state.lock().await;
            state.command_cache.insert(fp, output.clone());
        }
        output
    }

    async fn rg(
        &self,
        pattern: &str,
        path: &str,
        include: Vec<String>,
        exclude: Vec<String>,
    ) -> String {
        if pattern.trim().is_empty() {
            log::warn!("[fast-context] rg 缺少 pattern");
            return "Error: missing or invalid pattern".to_string();
        }
        let Ok(real_path) = self.real_path(path) else {
            log::warn!("[fast-context] rg 路径无法映射: {}", path);
            return format!("Error: path does not exist: {path}");
        };
        if !real_path.exists() {
            log::warn!("[fast-context] rg 路径不存在: {}", real_path.display());
            return format!("Error: path does not exist: {path}");
        }
        {
            let mut state = self.state.lock().await;
            state.collected_rg_patterns.push(pattern.to_string());
        }

        let mut command = Command::new("rg");
        command
            .arg("--no-heading")
            .arg("-n")
            .arg("--max-count")
            .arg("50");
        for glob in &include {
            command.arg("--glob").arg(glob);
        }
        for glob in &exclude {
            command.arg("--glob").arg(format!("!{glob}"));
        }
        command
            .arg(pattern)
            .arg(&real_path)
            .env("RIPGREP_CONFIG_PATH", "")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        match tokio::time::timeout(Duration::from_secs(30), command.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                let result = truncate_output(&self.remap(&String::from_utf8_lossy(&output.stdout)));
                log::info!(
                    "[fast-context] rg 成功: path={}, output_len={}",
                    real_path.display(),
                    result.len()
                );
                result
            }
            Ok(Ok(output)) if output.status.code() == Some(1) => {
                log::info!("[fast-context] rg 无匹配: pattern={}", pattern);
                "(no matches)".to_string()
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let result = truncate_output(&self.remap(if stderr.trim().is_empty() {
                    "Error: rg failed"
                } else {
                    &stderr
                }));
                log::warn!(
                    "[fast-context] rg 执行失败: status={:?}, output_len={}",
                    output.status.code(),
                    result.len()
                );
                result
            }
            // 本机未安装 rg 时走 Rust 内置搜索，保证 fast-context 不因外部二进制缺失不可用。
            Ok(Err(err)) => {
                log::warn!(
                    "[fast-context] 启动 rg 失败，改用 Rust 内置搜索: error={}",
                    err
                );
                self.rg_fallback(pattern, &real_path, &include, &exclude)
            }
            Err(_) => {
                log::warn!("[fast-context] rg 超时: pattern={}", pattern);
                "Error: rg timed out".to_string()
            }
        }
    }

    fn rg_fallback(
        &self,
        pattern: &str,
        real_path: &Path,
        include: &[String],
        exclude: &[String],
    ) -> String {
        let regex = match Regex::new(pattern) {
            Ok(regex) => regex,
            Err(err) => {
                log::warn!("[fast-context] Rust 内置搜索 regex 无效: {}", err);
                return format!("Error: invalid regex: {err}");
            }
        };
        let mut matches = Vec::new();
        collect_rg_matches(
            &self.root,
            real_path,
            &regex,
            include,
            exclude,
            &mut matches,
        );
        if matches.is_empty() {
            log::info!("[fast-context] Rust 内置搜索无匹配: pattern={}", pattern);
            "(no matches)".to_string()
        } else {
            let result = truncate_output(&self.remap(&matches.join("\n")));
            log::info!(
                "[fast-context] Rust 内置搜索完成: matches={}, output_len={}",
                matches.len(),
                result.len()
            );
            result
        }
    }

    async fn readfile(&self, file: &str, start_line: Option<usize>, end_line: Option<usize>) -> String {
        let Ok(path) = self.real_path(file) else {
            log::warn!("[fast-context] readfile 文件无法映射: {}", file);
            return format!("Error: file not found: {file}");
        };
        if !path.is_file() {
            log::warn!("[fast-context] readfile 文件不存在: {}", path.display());
            return format!("Error: file not found: {file}");
        }
        let key = normalize_path(&path);
        // #3 读文件缓存：同一 path 全量内容仅读盘一次，多次 readfile（不同 range）零额外 IO
        let content = {
            let state = self.state.lock().await;
            state.read_cache.get(&key).cloned()
        };
        let content = match content {
            Some(c) => c,
            None => match fs::read_to_string(&path) {
                Ok(c) => {
                    let mut state = self.state.lock().await;
                    state.read_cache.insert(key.clone(), c.clone());
                    c
                }
                Err(err) => {
                    log::warn!(
                        "[fast-context] readfile 读取失败: path={}, error={}",
                        path.display(),
                        err
                    );
                    return format!("Error: {err}");
                }
            },
        };
        let start = start_line.unwrap_or(1).max(1);
        let end = end_line
            .unwrap_or_else(|| content.lines().count())
            .max(start);
        let output = content
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let line_no = idx + 1;
                (line_no >= start && line_no <= end).then(|| format!("{line_no}:{line}"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_output(&output);
        log::info!(
            "[fast-context] readfile 完成: path={}, range={}-{}, output_len={}",
            path.display(),
            start,
            end,
            result.len()
        );
        result
    }

    fn tree(&self, path: &str, levels: Option<u8>) -> String {
        let Ok(real_path) = self.real_path(path) else {
            log::warn!("[fast-context] tree 目录无法映射: {}", path);
            return format!("Error: dir not found: {path}");
        };
        if !real_path.is_dir() {
            log::warn!("[fast-context] tree 目录不存在: {}", real_path.display());
            return format!("Error: dir not found: {path}");
        }
        let label = self.virtual_label(path);
        let result = truncate_output(&self.remap(&build_tree(
            &real_path,
            &label,
            levels.unwrap_or(3).clamp(1, 6),
            &[],
        )));
        log::info!(
            "[fast-context] tree 完成: path={}, output_len={}",
            real_path.display(),
            result.len()
        );
        result
    }

    fn ls(&self, path: &str, long_format: bool, all: bool) -> String {
        let Ok(real_path) = self.real_path(path) else {
            log::warn!("[fast-context] ls 目录无法映射: {}", path);
            return format!("Error: dir not found: {path}");
        };
        if !real_path.is_dir() {
            log::warn!("[fast-context] ls 不是目录: {}", real_path.display());
            return format!("Error: not a directory: {path}");
        }
        let mut entries = match fs::read_dir(&real_path) {
            Ok(entries) => entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
            Err(err) => {
                log::warn!(
                    "[fast-context] ls 读取目录失败: path={}, error={}",
                    real_path.display(),
                    err
                );
                return format!("Error: {err}");
            }
        };
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
        if !all {
            entries.retain(|entry| !entry.file_name().to_string_lossy().starts_with('.'));
        }
        if !long_format {
            let result = truncate_output(
                &entries
                    .iter()
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            log::info!(
                "[fast-context] ls 完成: path={}, entries={}, output_len={}",
                real_path.display(),
                entries.len(),
                result.len()
            );
            return result;
        }

        let mut lines = vec![format!("total {}", entries.len())];
        for entry in entries {
            let metadata = entry.metadata().ok();
            let kind = if metadata.as_ref().is_some_and(|m| m.is_dir()) {
                "d"
            } else {
                "-"
            };
            let size = metadata.map(|m| m.len()).unwrap_or(0);
            lines.push(format!(
                "{kind}rwxr-xr-x  1 user staff {size:>8} {}",
                entry.file_name().to_string_lossy()
            ));
        }
        let result = truncate_output(&lines.join("\n"));
        log::info!(
            "[fast-context] ls 长格式完成: path={}, output_len={}",
            real_path.display(),
            result.len()
        );
        result
    }

    fn glob(&self, pattern: &str, path: &str, type_filter: &str) -> String {
        if pattern.trim().is_empty() {
            log::warn!("[fast-context] glob 缺少 pattern");
            return "Error: missing or invalid pattern".to_string();
        }
        let Ok(root) = self.real_path(path) else {
            log::warn!("[fast-context] glob 路径无法映射: {}", path);
            return format!("Error: path does not exist: {path}");
        };
        if !root.exists() {
            log::warn!("[fast-context] glob 路径不存在: {}", root.display());
            return format!("Error: path does not exist: {path}");
        }
        let matcher = match GlobBuilder::new(pattern).literal_separator(true).build() {
            Ok(glob) => glob.compile_matcher(),
            Err(err) => {
                log::warn!("[fast-context] glob 表达式无效: {}", err);
                return format!("Error: invalid glob: {err}");
            }
        };
        let mut matches = Vec::new();
        collect_glob_matches(&root, &root, &matcher, type_filter, &mut matches);
        matches.sort();
        matches.truncate(100);
        if matches.is_empty() {
            log::info!("[fast-context] glob 无匹配: pattern={}", pattern);
            "(no matches)".to_string()
        } else {
            let result = self.remap(
                &matches
                    .iter()
                    .map(|path| normalize_path(path))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            log::info!(
                "[fast-context] glob 完成: matches={}, output_len={}",
                matches.len(),
                result.len()
            );
            result
        }
    }

    fn real_path(&self, value: &str) -> std::result::Result<PathBuf, ()> {
        if value.trim().is_empty() {
            return Err(());
        }
        let normalized = value.trim().replace('\\', "/");
        let candidate = if normalized.starts_with("/codebase") {
            let rel = normalized
                .trim_start_matches("/codebase")
                .trim_start_matches('/');
            let rel_path = Path::new(rel);
            if has_parent_dir(rel_path) {
                return Err(());
            }
            self.root.join(rel_path)
        } else {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                if has_parent_dir(&path) {
                    return Err(());
                }
                self.root.join(path)
            }
        };

        let absolute = candidate.canonicalize().unwrap_or(candidate);
        if absolute.starts_with(&self.root) {
            Ok(absolute)
        } else {
            Err(())
        }
    }

    fn remap(&self, text: &str) -> String {
        text.replace(&self.root_slash, "/codebase")
            .replace(&self.root.to_string_lossy().to_string(), "/codebase")
            .replace('\\', "/")
    }

    fn virtual_label(&self, path: &str) -> String {
        if path.trim().is_empty() {
            return "/codebase".to_string();
        }
        self.remap(path)
    }
}

/// 计算单个命令的指纹（用于缓存与重复检测）
fn command_fingerprint(cmd: &Value) -> String {
    let Some(kind) = cmd.get("type").and_then(Value::as_str) else {
        return String::new();
    };
    let canonical_strings = |key: &str| -> String {
        cmd.get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                let mut v: Vec<String> = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect();
                v.sort();
                v.join(",")
            })
            .unwrap_or_default()
    };
    match kind {
        "rg" => format!(
            "rg|{}|{}|{}|{}",
            cmd.get("pattern").and_then(Value::as_str).unwrap_or(""),
            cmd.get("path").and_then(Value::as_str).unwrap_or(""),
            canonical_strings("include"),
            canonical_strings("exclude"),
        ),
        "readfile" => format!(
            "readfile|{}|{:?}|{:?}",
            cmd.get("file").and_then(Value::as_str).unwrap_or(""),
            cmd.get("start_line").and_then(Value::as_u64),
            cmd.get("end_line").and_then(Value::as_u64),
        ),
        "tree" => format!(
            "tree|{}|{:?}",
            cmd.get("path").and_then(Value::as_str).unwrap_or(""),
            cmd.get("levels").and_then(Value::as_u64),
        ),
        "ls" => format!(
            "ls|{}|{}|{}",
            cmd.get("path").and_then(Value::as_str).unwrap_or(""),
            cmd.get("long_format").and_then(Value::as_bool).unwrap_or(false),
            cmd.get("all").and_then(Value::as_bool).unwrap_or(false),
        ),
        "glob" => format!(
            "glob|{}|{}|{}",
            cmd.get("pattern").and_then(Value::as_str).unwrap_or(""),
            cmd.get("path").and_then(Value::as_str).unwrap_or(""),
            cmd.get("type_filter").and_then(Value::as_str).unwrap_or("all"),
        ),
        _ => String::new(),
    }
}

fn collect_glob_matches(
    base: &Path,
    dir: &Path,
    matcher: &globset::GlobMatcher,
    type_filter: &str,
    matches: &mut Vec<PathBuf>,
) {
    if matches.len() >= 100 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        if matches.len() >= 100 {
            return;
        }
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let rel = path.strip_prefix(base).unwrap_or(&path);
        let name = entry.file_name().to_string_lossy().to_string();
        let matched = matcher.is_match(rel) || matcher.is_match(&name);
        if matched && matches_type(type_filter, metadata.is_file(), metadata.is_dir()) {
            matches.push(path.clone());
        }
        if metadata.is_dir() && !name.starts_with('.') {
            collect_glob_matches(base, &path, matcher, type_filter, matches);
        }
    }
}

fn collect_rg_matches(
    root: &Path,
    path: &Path,
    regex: &Regex,
    include: &[String],
    exclude: &[String],
    matches: &mut Vec<String>,
) {
    if matches.len() >= RESULT_MAX_LINES {
        return;
    }

    if path.is_file() {
        if !path_matches_filters(root, path, include, exclude) {
            return;
        }
        let Ok(content) = fs::read_to_string(path) else {
            return;
        };
        for (idx, line) in content.lines().enumerate() {
            if matches.len() >= RESULT_MAX_LINES {
                return;
            }
            if regex.is_match(line) {
                matches.push(format!("{}:{}:{}", normalize_path(path), idx + 1, line));
            }
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        if matches.len() >= RESULT_MAX_LINES {
            return;
        }
        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(".git") || exclude.iter().any(|pattern| glob_match(pattern, &name)) {
            continue;
        }
        if entry_path.is_dir() {
            collect_rg_matches(root, &entry_path, regex, include, exclude, matches);
        } else {
            collect_rg_matches(root, &entry_path, regex, include, exclude, matches);
        }
    }
}

fn path_matches_filters(root: &Path, path: &Path, include: &[String], exclude: &[String]) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_slash = normalize_path(rel);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    if exclude
        .iter()
        .any(|pattern| glob_match(pattern, &rel_slash) || glob_match(pattern, &file_name))
    {
        return false;
    }
    include.is_empty()
        || include
            .iter()
            .any(|pattern| glob_match(pattern, &rel_slash) || glob_match(pattern, &file_name))
}

fn matches_type(type_filter: &str, is_file: bool, is_dir: bool) -> bool {
    match type_filter {
        "file" => is_file,
        "directory" => is_dir,
        _ => true,
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn truncate_output(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let mut truncated = lines
        .iter()
        .take(RESULT_MAX_LINES)
        .map(|line| {
            if line.chars().count() > LINE_MAX_CHARS {
                line.chars().take(LINE_MAX_CHARS).collect::<String>()
            } else {
                (*line).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > RESULT_MAX_LINES {
        truncated.push_str("\n... (lines truncated) ...");
    }
    truncated
}

fn has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn ws_app_ver() -> String {
    env::var("WS_APP_VER").unwrap_or_else(|_| DEFAULT_WS_APP_VER.to_string())
}

fn ws_ls_ver() -> String {
    env::var("WS_LS_VER").unwrap_or_else(|_| DEFAULT_WS_LS_VER.to_string())
}

fn ws_model() -> String {
    env::var("WS_MODEL").unwrap_or_else(|_| DEFAULT_WS_MODEL.to_string())
}

fn system_info() -> Value {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "linux"
    };
    json!({
        "Os": os,
        "Arch": env::consts::ARCH,
        "Release": "",
        "Version": "",
        "Machine": env::consts::ARCH,
        "Nodename": hostname(),
        "Sysname": if cfg!(target_os = "macos") { "Darwin" } else if cfg!(target_os = "windows") { "Windows_NT" } else { "Linux" },
        "ProductVersion": ""
    })
}

fn cpu_info() -> Value {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    json!({
        "NumSockets": 1,
        "NumCores": threads,
        "NumThreads": threads,
        "VendorID": "",
        "Family": "0",
        "Model": "0",
        "ModelName": "Unknown",
        "Memory": 0
    })
}

fn hostname() -> String {
    env::var("COMPUTERNAME")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string())
}

#[allow(dead_code)]
fn jwt_exp(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value.get("exp").and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn protobuf_varint_round_trip() {
        for value in [0, 1, 127, 128, 16_384, u32::MAX as u64] {
            let bytes = encode_varint(value);
            let mut offset = 0;
            assert_eq!(decode_varint(&bytes, &mut offset), Some(value));
            assert_eq!(offset, bytes.len());
        }
    }

    #[test]
    fn connect_frame_round_trip_supports_gzip_and_plain() {
        let payload = b"hello fast-context";

        let compressed = connect_frame_encode(payload, true).expect("gzip frame 应可编码");
        assert_eq!(connect_frame_decode(&compressed), vec![payload.to_vec()]);

        let plain = connect_frame_encode(payload, false).expect("plain frame 应可编码");
        assert_eq!(connect_frame_decode(&plain), vec![payload.to_vec()]);
    }

    #[test]
    fn parse_tool_call_extracts_json_and_ignores_tail() {
        let parsed = parse_tool_call(
            "thinking\n[TOOL_CALLS]restricted_exec[ARGS]{\"command1\":{\"type\":\"rg\",\"pattern\":\"SouTool\",\"path\":\"/codebase/src\"}}</s>",
        )
        .expect("应识别 restricted_exec 调用");

        assert_eq!(parsed.thinking, "thinking");
        assert_eq!(parsed.name, "restricted_exec");
        assert_eq!(parsed.args["command1"]["pattern"].as_str(), Some("SouTool"));
    }

    #[test]
    fn parse_response_surfaces_connect_error_json() {
        let frame = connect_frame_encode(
            br#"{"error":{"code":"unauthenticated","message":"bad token"}}"#,
            false,
        )
        .expect("error frame 应可编码");
        let error = parse_response(&frame).expect_err("Connect error frame 应返回错误");
        assert!(error.to_string().contains("unauthenticated"));
        assert!(error.to_string().contains("bad token"));
    }

    #[test]
    fn parse_answer_keeps_safe_paths_and_rejects_escape() {
        let temp = tempdir().expect("临时目录应创建成功");
        let src_dir = temp.path().join("src");
        fs::create_dir_all(&src_dir).expect("src 目录应创建成功");
        let file = src_dir.join("lib.rs");
        fs::write(&file, "fn main() {}\n").expect("测试文件应写入成功");

        let xml = r#"
<ANSWER>
  <file path="/codebase/src/lib.rs"><range>1-10</range></file>
  <file path="/codebase/../secret.rs"><range>1-1</range></file>
</ANSWER>
"#;

        let files = parse_answer(xml, temp.path()).expect("ANSWER XML 应可解析");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path.as_deref(), Some("src/lib.rs"));
        assert_eq!(files[0].ranges, vec![[1, 10]]);
    }

    #[test]
    fn command_fingerprint_is_stable_and_order_insensitive() {
        // 同一 rg 命令，include 数组顺序不同也应得到相同指纹（避免假性重复缓存未命中）
        let cmd_a = json!({
            "type": "rg",
            "pattern": "ToolExecutor",
            "path": "/codebase/src",
            "include": ["*.rs", "*.toml"],
            "exclude": ["target"]
        });
        let cmd_b = json!({
            "type": "rg",
            "pattern": "ToolExecutor",
            "path": "/codebase/src",
            "include": ["*.toml", "*.rs"],
            "exclude": ["target"]
        });
        assert_eq!(command_fingerprint(&cmd_a), command_fingerprint(&cmd_b));

        // 不同 pattern 应得到不同指纹
        let cmd_c = json!({
            "type": "rg",
            "pattern": "OtherSymbol",
            "path": "/codebase/src",
            "include": ["*.rs"],
            "exclude": []
        });
        assert_ne!(command_fingerprint(&cmd_a), command_fingerprint(&cmd_c));

        // readfile 指纹应包含行范围
        let read_full = json!({"type": "readfile", "file": "/codebase/Cargo.toml"});
        let read_range = json!({
            "type": "readfile",
            "file": "/codebase/Cargo.toml",
            "start_line": 1,
            "end_line": 40
        });
        assert_ne!(command_fingerprint(&read_full), command_fingerprint(&read_range));
    }

    #[tokio::test]
    async fn tool_executor_caches_repeated_command_and_detects_duplicate() {
        let temp = tempdir().expect("临时目录应创建成功");
        // canonicalize 在 Windows 上会引入 \\?\ 前缀；提前对齐避免 starts_with 路径检查失败
        let temp_root = temp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| temp.path().to_path_buf());
        let lib = temp_root.join("lib.rs");
        fs::write(&lib, "fn alpha() {}\nfn beta() {}\n").expect("写入测试文件应成功");

        let executor = Arc::new(ToolExecutor::new(temp_root.clone()));

        // 同一个 readfile 命令调用一次（建立缓存与指纹）
        let args1 = json!({"command1": {"type": "readfile", "file": "/codebase/lib.rs"}});
        ToolExecutor::exec_tool_call(executor.clone(), &args1).await;

        // 第二轮：完全一致 → dup_count 应为 1
        let dup = executor.count_repeat_commands(&args1).await;
        assert_eq!(dup, 1, "完全一致的命令应被检测为重复");

        // 命令缓存命中验证：第二次同样调用应返回相同结果，且不产生新读盘（间接通过结果一致性验证）
        let args2 = args1.clone();
        let out_second = ToolExecutor::exec_tool_call(executor.clone(), &args2).await;
        assert!(
            out_second.contains("alpha"),
            "缓存命中后输出仍应包含原文件内容"
        );
    }
}
