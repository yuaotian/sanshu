//! 记忆管理器
//!
//! 核心记忆管理功能，包括：
//! - 记忆的添加、查询
//! - 启动时自动迁移和去重
//! - JSON 格式存储

use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::cleanup::{
    parse_backup_timestamp, preview_cleanup, BackupInfo, CleanupApplyRequest, CleanupApplyResult,
    CleanupPreviewRequest, CleanupPreviewResult, RestoreBackupResult,
};
use super::dedup::MemoryDeduplicator;
use super::migration::MemoryMigrator;
use super::similarity::TextSimilarity;
use super::types::{MemoryCategory, MemoryConfig, MemoryEntry, MemoryStore};
use crate::log_debug;

/// 记忆管理器
pub struct MemoryManager {
    /// 记忆目录路径
    memory_dir: PathBuf,
    /// 存储数据
    store: MemoryStore,
    /// 是否为非 Git 项目（降级模式）
    is_non_git_project: bool,
}

/// 路径规范化结果
struct NormalizeResult {
    /// 规范化后的路径
    path: PathBuf,
    /// 是否为非 Git 项目
    is_non_git: bool,
}

/// `add_memory` 的结果（方案 B：区分新增 / 同类更新 / 重复）
#[derive(Debug, Clone)]
pub enum AddOutcome {
    /// 新增了一条记忆，返回新条目 id
    Added(String),
    /// 就地更新了同类已有记忆（近义改写），返回被更新条目 id、相似度、旧内容
    Updated {
        id: String,
        similarity: f64,
        old_content: String,
    },
    /// 与已有记忆重复（≥ 去重阈值），静默拒绝
    Duplicate {
        similarity: f64,
        matched_content: Option<String>,
    },
}

impl MemoryManager {
    /// 存储文件名
    const STORE_FILE: &'static str = "memories.json";
    /// 运行时自动备份目录名
    const BACKUP_DIR: &'static str = "back";
    /// 最多保留的自动备份数量
    const MAX_BACKUPS: usize = 10;

    /// 创建新的记忆管理器
    ///
    /// 自动执行：
    /// 1. 路径规范化和验证（支持非 Git 项目降级）
    /// 2. 旧格式迁移（如果需要）
    /// 3. 启动时去重（如果配置启用）
    pub fn new(project_path: &str) -> Result<Self> {
        // 规范化项目路径（支持非 Git 项目降级）
        let normalize_result = Self::normalize_project_path(project_path)?;
        let memory_dir = normalize_result.path.join(".sanshu-memory");

        // 创建记忆目录
        fs::create_dir_all(&memory_dir).map_err(|e| {
            anyhow::anyhow!(
                "无法创建记忆目录: {}\n错误: {}\n这可能是因为项目目录没有写入权限。",
                Self::clean_display_path(&memory_dir),
                e
            )
        })?;

        let project_path_str = Self::clean_display_path(&normalize_result.path);

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
            let entries = std::mem::take(&mut store.entries);
            let (deduped, stats) = dedup.deduplicate(entries);

            if stats.removed_count > 0 {
                log_debug!(
                    "启动时去重: 移除 {} 条重复记忆，保留 {} 条",
                    stats.removed_count,
                    stats.remaining_count
                );
                store.last_dedup_at = Utc::now();
            }
            store.entries = deduped;
        }

        let manager = Self {
            memory_dir,
            store,
            is_non_git_project: normalize_result.is_non_git,
        };

        // 保存存储
        manager.save_store()?;

        Ok(manager)
    }

    /// 检查是否为非 Git 项目（降级模式）
    pub fn is_non_git_project(&self) -> bool {
        self.is_non_git_project
    }

    /// 添加记忆条目
    ///
    /// 如果启用了去重检测，会检查是否与现有记忆重复
    /// 重复时静默拒绝，返回 None
    pub fn add_memory(
        &mut self,
        content: &str,
        category: MemoryCategory,
    ) -> Result<Option<String>> {
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
        self.store
            .entries
            .iter()
            .filter(|e| e.category == category)
            .collect()
    }

    /// 添加记忆（方案 B：带同类 upsert 语义）
    ///
    /// 决策顺序：
    /// 1. 与**任意**记忆相似度 ≥ `similarity_threshold` → 判为重复，静默拒绝（沿用原去重）。
    /// 2. 否则，在**同一分类**内查找相似度 ≥ `upsert_threshold` 的最相似条目 →
    ///    就地更新其内容（近义改写合并），保留原 id 与 created_at，刷新 updated_at。
    /// 3. 否则 → 新增。
    ///
    /// 相比 [`add_memory`]，本方法从源头抑制「同一条规则的不同表述」堆积。
    pub fn upsert_memory(&mut self, content: &str, category: MemoryCategory) -> Result<AddOutcome> {
        let content = content.trim();
        if content.is_empty() {
            return Err(anyhow::anyhow!("记忆内容不能为空"));
        }

        // 1) 全局去重检查（≥ 去重阈值视为重复，静默拒绝）
        if self.store.config.enable_dedup {
            let dedup = MemoryDeduplicator::new(self.store.config.similarity_threshold);
            let dup_info = dedup.check_duplicate(content, &self.store.entries);
            if dup_info.is_duplicate {
                log_debug!(
                    "记忆去重: 相似度 {:.1}%，静默拒绝",
                    dup_info.similarity * 100.0
                );
                return Ok(AddOutcome::Duplicate {
                    similarity: dup_info.similarity,
                    matched_content: dup_info.matched_content,
                });
            }
        }

        // 2) 同类 upsert：在相同分类内寻找相似度落在
        //    [upsert_threshold, similarity_threshold) 区间的最相似条目
        let upsert_threshold = self.store.config.upsert_threshold;
        if upsert_threshold < self.store.config.similarity_threshold {
            let mut best: Option<(usize, f64)> = None;
            for (idx, entry) in self.store.entries.iter().enumerate() {
                if entry.category != category {
                    continue;
                }
                let sim = TextSimilarity::calculate_enhanced(content, &entry.content);
                if sim >= upsert_threshold {
                    match best {
                        Some((_, best_sim)) if sim <= best_sim => {}
                        _ => best = Some((idx, sim)),
                    }
                }
            }

            if let Some((idx, sim)) = best {
                let old_content = self.store.entries[idx].content.clone();
                let id = self.store.entries[idx].id.clone();
                let entry = &mut self.store.entries[idx];
                entry.content = content.to_string();
                entry.content_normalized = TextSimilarity::normalize(content);
                entry.updated_at = Utc::now();
                self.save_store()?;
                log_debug!(
                    "记忆同类更新(upsert): id={}, 相似度 {:.1}%",
                    id,
                    sim * 100.0
                );
                return Ok(AddOutcome::Updated {
                    id,
                    similarity: sim,
                    old_content,
                });
            }
        }

        // 3) 新增
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
        log_debug!("已新增记忆: {} ({:?})", id, category);
        Ok(AddOutcome::Added(id))
    }

    /// 手动执行去重
    ///
    /// 返回移除的记忆数量
    pub fn deduplicate(&mut self) -> Result<usize> {
        let dedup = MemoryDeduplicator::new(self.store.config.similarity_threshold);
        let (deduped, stats) = dedup.deduplicate(self.store.entries.clone());

        if stats.removed_count > 0 {
            self.create_backup("deduplicate")?;
        }
        self.store.entries = deduped;
        self.store.last_dedup_at = Utc::now();
        self.save_store()?;

        log_debug!("手动去重完成: 移除 {} 条重复记忆", stats.removed_count);
        Ok(stats.removed_count)
    }

    /// 执行去重并返回详细统计结果
    /// 用于前端可视化展示
    pub fn deduplicate_with_stats(&mut self) -> Result<super::dedup::DedupResult> {
        let dedup = MemoryDeduplicator::new(self.store.config.similarity_threshold);
        let (deduped, stats) = dedup.deduplicate(self.store.entries.clone());

        if stats.removed_count > 0 {
            self.create_backup("deduplicate")?;
        }
        self.store.entries = deduped;
        self.store.last_dedup_at = Utc::now();
        self.save_store()?;

        log_debug!("手动去重完成: 移除 {} 条重复记忆", stats.removed_count);
        Ok(stats)
    }

    /// 删除指定 ID 的记忆条目
    /// 返回被删除的记忆内容（用于确认）
    pub fn delete_memory(&mut self, memory_id: &str) -> Result<Option<String>> {
        if let Some(idx) = self
            .store
            .entries
            .iter()
            .position(|entry| entry.id == memory_id)
        {
            let deleted_content = self.store.entries[idx].content.clone();
            self.create_backup("delete")?;
            self.store.entries.remove(idx);
            log_debug!("已删除记忆: {}", memory_id);
            self.save_store()?;
            Ok(Some(deleted_content))
        } else {
            Ok(None) // 未找到该 ID
        }
    }

    /// 生成历史清理预览，不修改任何记忆。
    pub fn preview_cleanup(&self, request: CleanupPreviewRequest) -> CleanupPreviewResult {
        preview_cleanup(&self.store.entries, &request)
    }

    /// 应用用户确认后的历史清理计划。
    pub fn apply_cleanup_plan(
        &mut self,
        request: CleanupApplyRequest,
    ) -> Result<CleanupApplyResult> {
        let existing_ids: HashSet<String> = self
            .store
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        let mut remove_ids = HashSet::new();

        for group in &request.groups {
            if !existing_ids.contains(&group.keep_id) {
                return Err(anyhow::anyhow!(
                    "清理计划已过期：保留项不存在 ({})",
                    group.keep_id
                ));
            }

            for delete_id in &group.delete_ids {
                if delete_id == &group.keep_id {
                    continue;
                }
                if !existing_ids.contains(delete_id) {
                    return Err(anyhow::anyhow!(
                        "清理计划已过期：删除项不存在 ({})",
                        delete_id
                    ));
                }
                remove_ids.insert(delete_id.clone());
            }
        }

        if remove_ids.is_empty() {
            return Ok(CleanupApplyResult {
                backup_file: None,
                removed_count: 0,
                remaining_count: self.store.entries.len(),
                removed_ids: Vec::new(),
            });
        }

        let backup_file = if request.auto_backup {
            Some(self.create_backup("cleanup")?.file_name)
        } else {
            None
        };

        let mut removed_ids = Vec::new();
        self.store.entries.retain(|entry| {
            if remove_ids.contains(&entry.id) {
                removed_ids.push(entry.id.clone());
                false
            } else {
                true
            }
        });
        self.store.last_dedup_at = Utc::now();
        self.save_store()?;

        Ok(CleanupApplyResult {
            backup_file,
            removed_count: removed_ids.len(),
            remaining_count: self.store.entries.len(),
            removed_ids,
        })
    }

    /// 创建当前 memories.json 的运行时备份。
    pub fn create_backup(&self, reason: &str) -> Result<BackupInfo> {
        let backup_dir = self.backup_dir();
        fs::create_dir_all(&backup_dir)?;

        let source = self.store_path();
        if !source.exists() {
            self.save_store()?;
        }

        let now = Utc::now();
        let file_name = format!(
            "{}-{:03}.memories.json",
            now.format("%Y%m%d-%H%M%S"),
            now.timestamp_subsec_millis()
        );
        let backup_path = backup_dir.join(&file_name);
        fs::copy(&source, &backup_path).with_context(|| {
            format!(
                "创建记忆备份失败: {} -> {}",
                source.display(),
                backup_path.display()
            )
        })?;
        log_debug!("已创建记忆备份({}): {}", reason, backup_path.display());

        self.prune_backups(Self::MAX_BACKUPS)?;
        Self::backup_info_from_path(&backup_path)
    }

    /// 列出最近的记忆备份。
    pub fn list_backups(&self) -> Result<Vec<BackupInfo>> {
        let backup_dir = self.backup_dir();
        if !backup_dir.exists() {
            return Ok(Vec::new());
        }

        let mut backups = Vec::new();
        for entry in fs::read_dir(&backup_dir)? {
            let path = entry?.path();
            if path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with(".memories.json"))
                    .unwrap_or(false)
            {
                backups.push(Self::backup_info_from_path(&path)?);
            }
        }
        backups.sort_by(|a, b| b.file_name.cmp(&a.file_name));
        Ok(backups)
    }

    /// 读取备份文件内容，用于前端导出。
    pub fn export_backup(&self, file_name: &str) -> Result<String> {
        let backup_path = self.safe_backup_path(file_name)?;
        fs::read_to_string(&backup_path)
            .with_context(|| format!("读取备份失败: {}", backup_path.display()))
    }

    /// 恢复指定备份。恢复前会自动备份当前状态。
    pub fn restore_backup(&mut self, file_name: &str) -> Result<RestoreBackupResult> {
        let backup_path = self.safe_backup_path(file_name)?;
        let content = fs::read_to_string(&backup_path)
            .with_context(|| format!("读取备份失败: {}", backup_path.display()))?;
        let restored_store: MemoryStore = serde_json::from_str(&content)
            .with_context(|| format!("备份文件格式无效: {}", backup_path.display()))?;

        let safety_backup = self.create_backup("restore")?;
        self.store = restored_store;
        self.save_store()?;

        Ok(RestoreBackupResult {
            restored_file: file_name.to_string(),
            safety_backup_file: Some(safety_backup.file_name),
            entry_count: self.store.entries.len(),
        })
    }

    fn store_path(&self) -> PathBuf {
        self.memory_dir.join(Self::STORE_FILE)
    }

    fn backup_dir(&self) -> PathBuf {
        self.memory_dir.join(Self::BACKUP_DIR)
    }

    fn safe_backup_path(&self, file_name: &str) -> Result<PathBuf> {
        if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
            return Err(anyhow::anyhow!("备份文件名非法: {}", file_name));
        }
        let path = self.backup_dir().join(file_name);
        if !path.exists() {
            return Err(anyhow::anyhow!("备份不存在: {}", file_name));
        }
        Ok(path)
    }

    fn prune_backups(&self, max_count: usize) -> Result<()> {
        let mut backups = self.list_backups()?;
        if backups.len() <= max_count {
            return Ok(());
        }

        backups.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        let remove_count = backups.len() - max_count;
        for backup in backups.into_iter().take(remove_count) {
            let path = self.backup_dir().join(&backup.file_name);
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("删除旧备份失败: {}", path.display()))?;
                log_debug!("已清理旧记忆备份: {}", path.display());
            }
        }
        Ok(())
    }

    fn backup_info_from_path(path: &Path) -> Result<BackupInfo> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("备份文件名无效: {}", path.display()))?
            .to_string();
        let metadata = fs::metadata(path)?;
        let content = fs::read_to_string(path)?;
        let entry_count = serde_json::from_str::<MemoryStore>(&content)
            .map(|store| store.entries.len())
            .unwrap_or(0);
        let created_at = parse_backup_timestamp(&file_name)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| file_name.clone());

        Ok(BackupInfo {
            file_name,
            created_at,
            size_bytes: metadata.len(),
            entry_count,
        })
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
    // 以下是路径处理辅助方法
    // ========================================================================

    /// 清理 Windows 扩展路径前缀用于显示
    ///
    /// Windows 的 `canonicalize()` 会返回 `\\?\C:\...` 格式的路径，
    /// 这在错误消息和日志中显示不友好，需要清理前缀。
    fn clean_display_path(path: &Path) -> String {
        let path_str = path.to_string_lossy();
        // 处理 \\?\ 格式（Windows 扩展路径语法）
        if path_str.starts_with(r"\\?\") {
            return path_str[4..].to_string();
        }
        // 处理 //?/ 格式（canonicalize 在某些情况下返回）
        if path_str.starts_with("//?/") {
            return path_str[4..].to_string();
        }
        path_str.to_string()
    }

    /// 规范化项目路径
    ///
    /// 支持非 Git 项目降级：
    /// - 如果检测到 Git 仓库，使用 Git 根目录
    /// - 如果未检测到 Git 仓库，使用当前目录并标记为降级模式
    fn normalize_project_path(project_path: &str) -> Result<NormalizeResult> {
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
        let canonical_path = absolute_path.canonicalize().unwrap_or_else(|_| {
            // 如果 canonicalize 失败，尝试手动规范化
            Self::manual_canonicalize(&absolute_path).unwrap_or(absolute_path)
        });

        // 验证路径是否存在且为目录
        if !canonical_path.exists() {
            return Err(anyhow::anyhow!(
                "项目路径不存在: {}\n原始输入: {}\n规范化后: {}",
                Self::clean_display_path(&canonical_path),
                project_path,
                normalized_path_str
            ));
        }

        if !canonical_path.is_dir() {
            return Err(anyhow::anyhow!(
                "项目路径不是目录: {}",
                Self::clean_display_path(&canonical_path)
            ));
        }

        // 优先使用 git 根目录，否则降级使用当前目录
        if let Some(git_root) = Self::find_git_root(&canonical_path) {
            Ok(NormalizeResult {
                path: git_root,
                is_non_git: false,
            })
        } else {
            // 非 Git 项目降级：使用当前目录
            log_debug!(
                "未检测到 Git 仓库，使用项目目录作为记忆存储位置: {}",
                Self::clean_display_path(&canonical_path)
            );
            Ok(NormalizeResult {
                path: canonical_path,
                is_non_git: true,
            })
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

#[cfg(test)]
mod tests {
    use super::super::cleanup::CleanupApplyGroup;
    use super::*;
    use tempfile::TempDir;

    /// 在临时目录（非 Git，降级模式）创建管理器
    fn make_manager() -> (TempDir, MemoryManager) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let manager = MemoryManager::new(&path).unwrap();
        (dir, manager)
    }

    fn make_entry(
        id: &str,
        content: &str,
        category: MemoryCategory,
        updated_offset: i64,
    ) -> MemoryEntry {
        let now = Utc::now() + chrono::Duration::seconds(updated_offset);
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            content_normalized: TextSimilarity::normalize(content),
            category,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn test_upsert_added_then_duplicate() {
        let (_dir, mut m) = make_manager();

        // 首次新增
        let r = m
            .upsert_memory("使用 KISS 原则", MemoryCategory::Rule)
            .unwrap();
        assert!(matches!(r, AddOutcome::Added(_)), "首次应新增: {:?}", r);

        // 近乎相同（仅去空格）→ 应判为重复
        let r = m
            .upsert_memory("使用KISS原则", MemoryCategory::Rule)
            .unwrap();
        assert!(
            matches!(r, AddOutcome::Duplicate { .. }),
            "高相似应判重复: {:?}",
            r
        );

        // 只保留 1 条
        assert_eq!(m.get_all_memories().len(), 1);
    }

    #[test]
    fn test_upsert_updates_same_category_paraphrase() {
        let (_dir, mut m) = make_manager();

        // 为确定性地验证 upsert 路径：把去重阈值抬高到 0.90，upsert 阈值 0.55。
        // 这样相似度 0.84 的近义改写落入 [0.55, 0.90) 的 upsert 区间，
        // 应触发就地更新，而非被全局去重拦截。
        m.update_config(MemoryConfig {
            similarity_threshold: 0.90,
            dedup_on_startup: true,
            enable_dedup: true,
            upsert_threshold: 0.55,
        })
        .unwrap();

        let a = "用户偏好：复杂审计/修复任务完成后生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先使用 sou 做代码语义搜索；关键确认使用 zhi。";
        let r = m.upsert_memory(a, MemoryCategory::Preference).unwrap();
        let first_id = match r {
            AddOutcome::Added(id) => id,
            other => panic!("首次应新增: {:?}", other),
        };

        // 同类、近义改写（相似度约 0.84，落入 upsert 区间）→ 应就地更新
        let b = "用户偏好：复杂审计/修复任务需生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先用 sou 做代码语义搜索并查看上下文；关键确认通过 zhi 展示。";
        let r = m.upsert_memory(b, MemoryCategory::Preference).unwrap();
        match r {
            AddOutcome::Updated { id, .. } => {
                assert_eq!(id, first_id, "应更新同一条目");
            }
            other => panic!("同类近义改写应更新而非新增: {:?}", other),
        }

        // 更新而非新增：仍只有 1 条，且内容已是新表述
        let all = m.get_all_memories();
        assert_eq!(all.len(), 1, "同类近义改写不应增加条目数");
        assert_eq!(all[0].content, b, "内容应更新为最新表述");
    }

    #[test]
    fn test_strong_paraphrase_deduped_by_scheme_a() {
        // 方案 A + B 协同：默认阈值下，相似度 0.84 的强近义改写会先被
        // 全局去重（≥0.70）拦截为 Duplicate。无论 Duplicate 还是 Updated，
        // 目标都达成——近义改写不产生新条目，条目数保持为 1。
        let (_dir, mut m) = make_manager();

        let a = "用户偏好：复杂审计/修复任务完成后生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先使用 sou 做代码语义搜索；关键确认使用 zhi。";
        m.upsert_memory(a, MemoryCategory::Preference).unwrap();

        let b = "用户偏好：复杂审计/修复任务需生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先用 sou 做代码语义搜索并查看上下文；关键确认通过 zhi 展示。";
        let r = m.upsert_memory(b, MemoryCategory::Preference).unwrap();
        assert!(
            matches!(r, AddOutcome::Duplicate { .. } | AddOutcome::Updated { .. }),
            "近义改写不应新增: {:?}",
            r
        );
        assert_eq!(m.get_all_memories().len(), 1, "近义改写不应增加条目数");
    }

    #[test]
    fn test_upsert_different_category_adds() {
        let (_dir, mut m) = make_manager();

        // 内容相近但分类不同 → 不触发 upsert，应各自新增
        let text = "不要生成测试脚本，不要编译，不要运行";
        m.upsert_memory(text, MemoryCategory::Rule).unwrap();
        let r = m.upsert_memory(text, MemoryCategory::Context).unwrap();
        // 注意：跨分类，但全局去重仍会先拦截高相似内容
        // 这里内容完全相同 → 会被全局去重判为重复（符合预期，避免跨类重复）
        assert!(
            matches!(r, AddOutcome::Duplicate { .. }),
            "完全相同内容应被全局去重拦截: {:?}",
            r
        );
    }

    #[test]
    fn test_upsert_unrelated_adds_new() {
        let (_dir, mut m) = make_manager();

        m.upsert_memory("使用 KISS 原则", MemoryCategory::Rule)
            .unwrap();
        let r = m
            .upsert_memory("配置数据库连接池大小为 20", MemoryCategory::Rule)
            .unwrap();
        assert!(
            matches!(r, AddOutcome::Added(_)),
            "不相关内容应新增: {:?}",
            r
        );
        assert_eq!(m.get_all_memories().len(), 2);
    }

    #[test]
    fn test_cleanup_apply_creates_backup_and_removes_selected() {
        let (_dir, mut m) = make_manager();
        m.store.entries = vec![
            make_entry("a", "不要编译，用户自己编译", MemoryCategory::Rule, 0),
            make_entry("b", "禁止编译，用户自己编译", MemoryCategory::Rule, 10),
            make_entry("c", "配置数据库连接池大小", MemoryCategory::Rule, 0),
        ];
        m.save_store().unwrap();

        let preview = m.preview_cleanup(CleanupPreviewRequest {
            threshold: 0.55,
            categories: vec!["规范".to_string()],
            include_cross_category: false,
        });
        assert_eq!(preview.candidate_group_count, 1);
        let group = &preview.groups[0];

        let result = m
            .apply_cleanup_plan(CleanupApplyRequest {
                auto_backup: true,
                groups: vec![CleanupApplyGroup {
                    group_id: group.group_id.clone(),
                    keep_id: group.recommended_keep_id.clone(),
                    delete_ids: group.default_delete_ids.clone(),
                }],
            })
            .unwrap();

        assert_eq!(result.removed_count, 1);
        assert!(result.backup_file.is_some());
        assert_eq!(m.get_all_memories().len(), 2);
        assert_eq!(m.list_backups().unwrap().len(), 1);
    }

    #[test]
    fn test_backup_retention_keeps_latest_ten() {
        let (_dir, m) = make_manager();

        for _ in 0..12 {
            m.create_backup("retention").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let backups = m.list_backups().unwrap();
        assert_eq!(backups.len(), 10);
    }

    #[test]
    fn test_restore_backup_creates_safety_backup() {
        let (_dir, mut m) = make_manager();
        m.store.entries = vec![make_entry("a", "使用 KISS 原则", MemoryCategory::Rule, 0)];
        m.save_store().unwrap();
        let backup = m.create_backup("before-change").unwrap();

        m.store.entries.push(make_entry(
            "b",
            "配置数据库连接池大小",
            MemoryCategory::Rule,
            0,
        ));
        m.save_store().unwrap();

        let result = m.restore_backup(&backup.file_name).unwrap();

        assert_eq!(result.restored_file, backup.file_name);
        assert!(result.safety_backup_file.is_some());
        assert_eq!(result.entry_count, 1);
        assert_eq!(m.get_all_memories().len(), 1);
    }
}
