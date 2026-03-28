// 对话历史管理模块
// 持久化存储用户与弹窗的交互历史，供提示词增强时使用

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ring::digest::{Context as ShaContext, SHA256};

use crate::{log_debug, log_important};
use crate::mcp::utils::safe_truncate;

/// 对话历史管理器
pub struct ChatHistoryManager {
    project_hash: String,
    project_path: String,
    max_entries: usize,
}

/// 单条对话历史
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    pub id: String,
    pub user_input: String,
    pub ai_response_summary: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub source: String,
}

/// 历史文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ChatHistoryFile {
    #[serde(default)]
    project_path: String,
    #[serde(default)]
    entries: VecDeque<ChatEntry>,
    #[serde(default)]
    last_updated: Option<DateTime<Utc>>,
}

impl ChatHistoryManager {
    const DEFAULT_MAX_ENTRIES: usize = 20;

    pub fn new(project_path: &str) -> Result<Self> {
        let project_hash = Self::hash_path(project_path);
        Ok(Self {
            project_hash,
            project_path: project_path.to_string(),
            max_entries: Self::DEFAULT_MAX_ENTRIES,
        })
    }

    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }

    /// 清理 Windows 长路径前缀 + 统一分隔符 + 去除末尾斜杠 + 小写
    fn normalize_path(path: &str) -> String {
        let mut p = path.trim().to_string();

        if p.starts_with(r"\\?\") {
            p = p[4..].to_string();
        }
        if p.starts_with("//?/") {
            p = p[4..].to_string();
        }

        p = p.replace('\\', "/");

        if p.starts_with("//?/") {
            p = p[4..].to_string();
        }

        p = p.trim_end_matches('/').to_string();
        p.to_lowercase()
    }

    fn sha256_short_hex(input: &str) -> String {
        let mut ctx = ShaContext::new(&SHA256);
        ctx.update(input.as_bytes());
        let digest = ctx.finish();
        hex::encode(&digest.as_ref()[..8])
    }

    fn hash_path(path: &str) -> String {
        Self::sha256_short_hex(&Self::normalize_path(path))
    }

    fn history_dir() -> PathBuf {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".sanshu")
            .join("chat_history");
        let _ = fs::create_dir_all(&data_dir);
        data_dir
    }

    fn history_file_path(&self) -> PathBuf {
        Self::history_dir().join(format!("{}.json", self.project_hash))
    }

    fn empty_history(&self) -> ChatHistoryFile {
        ChatHistoryFile {
            project_path: self.project_path.clone(),
            entries: VecDeque::new(),
            last_updated: None,
        }
    }

    fn load_history(&self) -> Result<ChatHistoryFile> {
        let path = self.history_file_path();
        if !path.exists() {
            return Ok(self.empty_history());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("读取对话历史文件失败: {}", path.display()))?;
        let parsed: ChatHistoryFile = serde_json::from_str(&content)
            .with_context(|| format!("解析对话历史文件失败: {}", path.display()))?;
        Ok(parsed)
    }

    fn save_history(&self, history: &ChatHistoryFile) -> Result<()> {
        let path = self.history_file_path();
        let content = serde_json::to_string_pretty(history)?;
        fs::write(&path, content)
            .with_context(|| format!("写入对话历史文件失败: {}", path.display()))?;
        log_debug!("对话历史已保存: {}", path.display());
        Ok(())
    }

    pub fn add_entry(&self, user_input: &str, ai_response: &str, source: &str) -> Result<String> {
        let mut history = self.load_history()?;

        let id = format!("{}_{}", 
            chrono::Utc::now().timestamp_millis(),
            fastrand::u32(..)
        );

        let ai_summary = safe_truncate(ai_response, 500);

        let entry = ChatEntry {
            id: id.clone(),
            user_input: user_input.to_string(),
            ai_response_summary: ai_summary,
            timestamp: Utc::now(),
            source: source.to_string(),
        };

        history.entries.push_back(entry);
        
        while history.entries.len() > self.max_entries {
            history.entries.pop_front();
        }

        history.last_updated = Some(Utc::now());
        self.save_history(&history)?;

        log_important!(info, "对话历史已记录: id={}, source={}", id, source);
        Ok(id)
    }

    pub fn get_recent(&self, count: usize) -> Result<Vec<ChatEntry>> {
        let history = self.load_history()?;
        let entries: Vec<ChatEntry> = history.entries.into_iter().collect();
        if entries.len() <= count {
            return Ok(entries);
        }
        Ok(entries
            .into_iter()
            .rev()
            .take(count)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect())
    }

    pub fn get_recent_entries(&self, count: usize) -> Result<Vec<ChatEntry>> {
        self.get_recent(count)
    }

    pub fn get_all(&self) -> Result<Vec<ChatEntry>> {
        let history = self.load_history()?;
        Ok(history.entries.into_iter().collect())
    }

    pub fn get_by_ids(&self, ids: &[String]) -> Result<Vec<ChatEntry>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let history = self.load_history()?;
        let mut map: HashMap<String, ChatEntry> = HashMap::new();
        for entry in history.entries {
            map.insert(entry.id.clone(), entry);
        }

        Ok(ids.iter()
            .filter_map(|id| map.get(id).cloned())
            .collect())
    }

    pub fn clear(&self) -> Result<()> {
        let history = ChatHistoryFile {
            project_path: self.project_path.clone(),
            entries: VecDeque::new(),
            last_updated: Some(Utc::now()),
        };
        self.save_history(&history)?;
        log_important!(info, "对话历史已清空: project={}", self.project_path);
        Ok(())
    }

    pub fn remove_entry(&self, entry_id: &str) -> Result<bool> {
        let path = self.history_file_path();
        if !path.exists() {
            return Ok(false);
        }

        let mut history = self.load_history()?;
        let original_len = history.entries.len();
        history.entries.retain(|e| e.id != entry_id);

        if history.entries.len() < original_len {
            history.last_updated = Some(Utc::now());
            self.save_history(&history)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn to_api_format(&self, count: usize) -> Result<Vec<super::types::ChatHistoryEntry>> {
        let entries = self.get_recent(count)?;

        Ok(entries.into_iter().map(|entry| {
            super::types::ChatHistoryEntry {
                request_message: entry.user_input.clone(),
                request_id: entry.id.clone(),
                request_nodes: vec![
                    super::types::ChatHistoryRequestNode {
                        id: 0,
                        node_type: 0,
                        text_node: Some(super::types::TextNode {
                            content: entry.user_input,
                        }),
                    }
                ],
                response_nodes: vec![
                    super::types::ChatHistoryResponseNode {
                        id: 1,
                        node_type: 0,
                        content: Some(entry.ai_response_summary),
                        tool_use: None,
                        thinking: None,
                        billing_metadata: None,
                        metadata: None,
                        token_usage: None,
                    }
                ],
            }
        }).collect())
    }

    pub fn to_api_format_by_ids(&self, ids: &[String]) -> Result<Vec<super::types::ChatHistoryEntry>> {
        let entries = self.get_by_ids(ids)?;

        Ok(entries.into_iter().map(|entry| {
            super::types::ChatHistoryEntry {
                request_message: entry.user_input.clone(),
                request_id: entry.id.clone(),
                request_nodes: vec![
                    super::types::ChatHistoryRequestNode {
                        id: 0,
                        node_type: 0,
                        text_node: Some(super::types::TextNode {
                            content: entry.user_input,
                        }),
                    }
                ],
                response_nodes: vec![
                    super::types::ChatHistoryResponseNode {
                        id: 1,
                        node_type: 0,
                        content: Some(entry.ai_response_summary),
                        tool_use: None,
                        thinking: None,
                        billing_metadata: None,
                        metadata: None,
                        token_usage: None,
                    }
                ],
            }
        }).collect())
    }
}
