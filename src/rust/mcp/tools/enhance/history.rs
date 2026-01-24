// 对话历史管理模块
// 持久化存储用户与弹窗的交互历史，供提示词增强时使用

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

use crate::{log_debug, log_important};

/// 对话历史管理器
pub struct ChatHistoryManager {
    /// 项目根路径的哈希值（用于文件名）
    project_hash: String,
    /// 原始项目路径
    project_path: String,
    /// 最大历史条数
    max_entries: usize,
}

/// 单条对话历史
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    /// 唯一ID
    pub id: String,
    /// 用户输入
    pub user_input: String,
    /// AI响应摘要（仅保存前500字符）
    pub ai_response_summary: String,
    /// 时间戳
    pub timestamp: DateTime<Utc>,
    /// 来源: "popup" | "mcp" | "telegram"
    pub source: String,
}

/// 历史文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ChatHistoryFile {
    /// 项目路径
    project_path: String,
    /// 对话历史列表
    entries: VecDeque<ChatEntry>,
    /// 最后更新时间
    last_updated: Option<DateTime<Utc>>,
}

impl ChatHistoryManager {
    /// 最大历史条数默认值
    const DEFAULT_MAX_ENTRIES: usize = 20;

    /// 创建对话历史管理器
    pub fn new(project_path: &str) -> Result<Self> {
        let project_hash = Self::hash_path(project_path);
        Ok(Self {
            project_hash,
            project_path: project_path.to_string(),
            max_entries: Self::DEFAULT_MAX_ENTRIES,
        })
    }

    /// 设置最大历史条数
    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }

    /// 计算路径哈希
    fn hash_path(path: &str) -> String {
        let normalized = path
            .trim()
            .to_lowercase()
            .replace('\\', "/");
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        hex::encode(&hasher.finalize()[..8]) // 取前8字节作为短哈希
    }

    /// 获取历史文件路径
    fn history_file_path(&self) -> PathBuf {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".sanshu")
            .join("chat_history");
        
        // 确保目录存在
        let _ = fs::create_dir_all(&data_dir);
        
        data_dir.join(format!("{}.json", self.project_hash))
    }

    /// 加載历史文件
    fn load_history(&self) -> ChatHistoryFile {
        let path = self.history_file_path();
        if !path.exists() {
            return ChatHistoryFile {
                project_path: self.project_path.clone(),
                entries: VecDeque::new(),
                last_updated: None,
            };
        }

        match fs::read_to_string(&path) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_else(|e| {
                    log_debug!("解析对话历史文件失败: {}", e);
                    ChatHistoryFile {
                        project_path: self.project_path.clone(),
                        entries: VecDeque::new(),
                        last_updated: None,
                    }
                })
            }
            Err(e) => {
                log_debug!("读取对话历史文件失败: {}", e);
                ChatHistoryFile {
                    project_path: self.project_path.clone(),
                    entries: VecDeque::new(),
                    last_updated: None,
                }
            }
        }
    }

    /// 保存历史文件
    fn save_history(&self, history: &ChatHistoryFile) -> Result<()> {
        let path = self.history_file_path();
        let content = serde_json::to_string_pretty(history)?;
        fs::write(&path, content)?;
        log_debug!("对话历史已保存: {}", path.display());
        Ok(())
    }

    /// 添加一条对话记录
    pub fn add_entry(&self, user_input: &str, ai_response: &str, source: &str) -> Result<String> {
        let mut history = self.load_history();
        
        // 生成唯一ID
        let id = format!("{}_{}", 
            chrono::Utc::now().timestamp_millis(),
            fastrand::u32(..)
        );

        // 截取AI响应摘要（最多500字符）
        let ai_summary = if ai_response.len() > 500 {
            format!("{}...", &ai_response[..500])
        } else {
            ai_response.to_string()
        };

        let entry = ChatEntry {
            id: id.clone(),
            user_input: user_input.to_string(),
            ai_response_summary: ai_summary,
            timestamp: Utc::now(),
            source: source.to_string(),
        };

        history.entries.push_back(entry);
        
        // 保持历史条数在限制内
        while history.entries.len() > self.max_entries {
            history.entries.pop_front();
        }

        history.last_updated = Some(Utc::now());
        self.save_history(&history)?;

        log_important!(info, "对话历史已记录: id={}, source={}", id, source);
        Ok(id)
    }

    /// 获取最近N条对话历史
    pub fn get_recent(&self, count: usize) -> Vec<ChatEntry> {
        let history = self.load_history();
        history.entries
            .iter()
            .rev()
            .take(count)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// 获取所有对话历史
    pub fn get_all(&self) -> Vec<ChatEntry> {
        let history = self.load_history();
        history.entries.into_iter().collect()
    }

    /// 清空对话历史
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

    /// 删除指定ID的历史条目
    pub fn remove_entry(&self, entry_id: &str) -> Result<bool> {
        let mut history = self.load_history();
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

    /// 转换为 chat-stream API 所需的格式
    pub fn to_api_format(&self, count: usize) -> Vec<super::types::ChatHistoryEntry> {
        let entries = self.get_recent(count);
        
        entries.into_iter().enumerate().map(|(idx, entry)| {
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
        }).collect()
    }
}
