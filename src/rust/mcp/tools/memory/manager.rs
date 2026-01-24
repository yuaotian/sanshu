//! 记忆管理器
//!
//! 核心记忆管理功能，包括：
//! - 记忆的添加、查询
//! - 启动时自动迁移和去重
//! - JSON 格式存储

use anyhow::Result;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

use super::types::{MemoryEntry, MemoryCategory, MemoryStore, MemoryConfig};
use super::similarity::TextSimilarity;
use super::dedup::MemoryDeduplicator;
use super::migration::MemoryMigrator;
use crate::log_debug;

/// 记忆管理器
pub struct MemoryManager {
    /// 记忆目录路径
    memory_dir: PathBuf,
    /// 项目路径
    project_path: String,
    /// 存储数据
    store: MemoryStore,
}

impl MemoryManager {
    /// 存储文件名
    const STORE_FILE: &'static str = "memories.json";

    /// 创建新的记忆管理器
    ///
    /// 自动执行：
    /// 1. 路径规范化和验证
    /// 2. 旧格式迁移（如果需要）
    /// 3. 启动时去重（如果配置启用）
    pub fn new(project_path: &str) -> Result<Self> {
        // 规范化项目路径
        let normalized_path = Self::normalize_project_path(project_path)?;
        let memory_dir = normalized_path.join(".sanshu-memory");

        // 创建记忆目录
        fs::create_dir_all(&memory_dir)
            .map_err(|e| anyhow::anyhow!(
                "无法在git项目中创建记忆目录: {}\n错误: {}\n这可能是因为项目目录没有写入权限。",
                memory_dir.display(),
                e
            ))?;

        let project_path_str = normalized_path.to_string_lossy().to_string();

        // 检查是否需要迁移
        if MemoryMigrator::needs_migration(&memory_dir) {
            log_debug!("检测到旧版记忆格式，开始迁移...");
            match MemoryMigrator::migrate(&memory_dir, &project_path_str) {
                Ok(result) => {
                    log_debug!(
                        "迁移完成: 读取 {} 条，去重后 {} 条，移除 {} 条重复",
                        result.md_entries_count,
                        result.deduped_entries_count,
                        result.removed_duplicates
                    );
                }
                Err(e) => {
                    log_debug!("迁移失败（将使用空存储）: {}", e);
                }
            }
        }

        // 加载或创建存储
        let store_path = memory_dir.join(Self::STORE_FILE);
        let mut store = if store_path.exists() {
            let content = fs::read_to_string(&store_path)?;
            serde_json::from_str(&content).unwrap_or_else(|e| {
                log_debug!("解析存储文件失败，使用默认值: {}", e);
                MemoryStore {
                    project_path: project_path_str.clone(),
                    ..Default::default()
                }
            })
        } else {
            MemoryStore {
                project_path: project_path_str.clone(),
                ..Default::default()
            }
        };

        // 如果配置启用了启动时去重，执行去重
        if store.config.dedup_on_startup && !store.entries.is_empty() {
            let dedup = MemoryDeduplicator::new(store.config.similarity_threshold);
            let (deduped, stats) = dedup.deduplicate(store.entries);

            if stats.removed_count > 0 {
                log_debug!(
                    "启动时去重: 移除 {} 条重复记忆，保留 {} 条",
                    stats.removed_count,
                    stats.remaining_count
                );
                store.entries = deduped;
                store.last_dedup_at = Utc::now();
            }
        }

        let mut manager = Self {
            memory_dir,
            project_path: project_path_str,
            store,
        };

        // 保存存储
        manager.save_store()?;

        Ok(manager)
    }

    /// 添加记忆条目
    ///
    /// 如果启用了去重检测，会检查是否与现有记忆重复
    /// 重复时静默拒绝，返回 None
    pub fn add_memory(&mut self, content: &str, category: MemoryCategory) -> Result<Option<String>> {
        let content = content.trim();
        if content.is_empty() {
            return Err(anyhow::anyhow!("记忆内容不能为空"));
        }

        // 如果启用去重检测，检查是否重复
        if self.store.config.enable_dedup {
            let dedup = MemoryDeduplicator::new(self.store.config.similarity_threshold);
            let dup_info = dedup.check_duplicate(content, &self.store.entries);

            if dup_info.is_duplicate {
                log_debug!(
                    "记忆去重: 新内容与现有记忆相似度 {:.1}%，静默拒绝。匹配内容: {:?}",
                    dup_info.similarity * 100.0,
                    dup_info.matched_content
                );
                return Ok(None); // 静默拒绝，不报错
            }
        }

        // 创建新记忆条目
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            content_normalized: TextSimilarity::normalize(content),
            category,
            created_at: now,
            updated_at: now,
        };

        self.store.entries.push(entry);
        self.save_store()?;

        log_debug!("已添加记忆: {} ({:?})", id, category);
        Ok(Some(id))
    }

    /// 获取所有记忆
    pub fn get_all_memories(&self) -> Vec<&MemoryEntry> {
        self.store.entries.iter().collect()
    }

    /// 获取指定分类的记忆
    pub fn get_memories_by_category(&self, category: MemoryCategory) -> Vec<&MemoryEntry> {
        self.store.entries
            .iter()
            .filter(|e| e.category == category)
            .collect()
    }

    /// 手动执行去重
    ///
    /// 返回移除的记忆数量
    pub fn deduplicate(&mut self) -> Result<usize> {
        let dedup = MemoryDeduplicator::new(self.store.config.similarity_threshold);
        let (deduped, stats) = dedup.deduplicate(std::mem::take(&mut self.store.entries));

        self.store.entries = deduped;
        self.store.last_dedup_at = Utc::now();
        self.save_store()?;

        log_debug!("手动去重完成: 移除 {} 条重复记忆", stats.removed_count);
        Ok(stats.removed_count)
    }

    /// 获取记忆统计信息
    pub fn get_stats(&self) -> MemoryStats {
        let mut stats = MemoryStats::default();
        stats.total = self.store.entries.len();

        for entry in &self.store.entries {
            match entry.category {
                MemoryCategory::Rule => stats.rules += 1,
                MemoryCategory::Preference => stats.preferences += 1,
                MemoryCategory::Pattern => stats.patterns += 1,
                MemoryCategory::Context => stats.contexts += 1,
            }
        }

        stats
    }

    /// 获取项目信息供MCP调用方分析 - 压缩简化版本
    pub fn get_project_info(&self) -> String {
        if self.store.entries.is_empty() {
            return "📭 暂无项目记忆".to_string();
        }

        let mut compressed_info = Vec::new();

        // 按分类压缩汇总
        let categories = [
            (MemoryCategory::Rule, "规范"),
            (MemoryCategory::Preference, "偏好"),
            (MemoryCategory::Pattern, "模式"),
            (MemoryCategory::Context, "背景"),
        ];

        for (category, title) in categories.iter() {
            let memories: Vec<_> = self.get_memories_by_category(*category);
            if !memories.is_empty() {
                let items: Vec<String> = memories
                    .iter()
                    .map(|m| {
                        // 去除多余空格和换行，压缩内容
                        m.content
                            .split_whitespace()
                            .collect::<Vec<&str>>()
                            .join(" ")
                    })
                    .filter(|s| !s.is_empty())
                    .collect();

                if !items.is_empty() {
                    compressed_info.push(format!("**{}**: {}", title, items.join("; ")));
                }
            }
        }

        if compressed_info.is_empty() {
            "📭 暂无有效项目记忆".to_string()
        } else {
            format!("📚 项目记忆总览: {}", compressed_info.join(" | "))
        }
    }

    /// 获取去重配置
    pub fn config(&self) -> &MemoryConfig {
        &self.store.config
    }

    /// 更新去重配置
    pub fn update_config(&mut self, config: MemoryConfig) -> Result<()> {
        self.store.config = config;
        self.save_store()
    }

    /// 保存存储到文件
    fn save_store(&self) -> Result<()> {
        let store_path = self.memory_dir.join(Self::STORE_FILE);
        let json = serde_json::to_string_pretty(&self.store)?;
        fs::write(&store_path, json)?;
        Ok(())
    }

    // ========================================================================
    // 以下是路径处理辅助方法（保持原有逻辑）
    // ========================================================================

    /// 规范化项目路径
    fn normalize_project_path(project_path: &str) -> Result<PathBuf> {
        // 使用增强的路径解码和规范化功能
        let normalized_path_str = crate::mcp::utils::decode_and_normalize_path(project_path)
            .map_err(|e| anyhow::anyhow!("路径格式错误: {}", e))?;

        let path = Path::new(&normalized_path_str);

        // 转换为绝对路径
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // 规范化路径（解析 . 和 .. 等）
        let canonical_path = absolute_path.canonicalize()
            .unwrap_or_else(|_| {
                // 如果 canonicalize 失败，尝试手动规范化
                Self::manual_canonicalize(&absolute_path).unwrap_or(absolute_path)
            });

        // 验证路径是否存在且为目录
        if !canonical_path.exists() {
            return Err(anyhow::anyhow!(
                "项目路径不存在: {}\n原始输入: {}\n规范化后: {}",
                canonical_path.display(),
                project_path,
                normalized_path_str
            ));
        }

        if !canonical_path.is_dir() {
            return Err(anyhow::anyhow!("项目路径不是目录: {}", canonical_path.display()));
        }

        // 验证是否为 git 根目录或其子目录
        if let Some(git_root) = Self::find_git_root(&canonical_path) {
            Ok(git_root)
        } else {
            Err(anyhow::anyhow!(
                "错误：提供的项目路径不在 git 仓库中。\n路径: {}\n请确保在 git 根目录（包含 .git 文件夹的目录）中调用此功能。",
                canonical_path.display()
            ))
        }
    }

    /// 手动规范化路径
    fn manual_canonicalize(path: &Path) -> Result<PathBuf> {
        let mut components = Vec::new();

        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if !components.is_empty() {
                        components.pop();
                    }
                }
                _ => {
                    components.push(component);
                }
            }
        }

        let mut result = PathBuf::new();
        for component in components {
            result.push(component);
        }

        Ok(result)
    }

    /// 查找 git 根目录
    fn find_git_root(start_path: &Path) -> Option<PathBuf> {
        let mut current_path = start_path;

        loop {
            let git_path = current_path.join(".git");
            if git_path.exists() {
                return Some(current_path.to_path_buf());
            }

            match current_path.parent() {
                Some(parent) => current_path = parent,
                None => break,
            }
        }

        None
    }
}

/// 记忆统计信息
#[derive(Debug, Default)]
pub struct MemoryStats {
    pub total: usize,
    pub rules: usize,
    pub preferences: usize,
    pub patterns: usize,
    pub contexts: usize,
}
