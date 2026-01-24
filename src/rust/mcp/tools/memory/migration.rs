//! 记忆格式迁移模块
//!
//! 将旧版 Markdown 格式迁移到新版 JSON 格式

use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use chrono::Utc;

use super::types::{MemoryEntry, MemoryCategory, MemoryStore, MemoryConfig};
use super::dedup::MemoryDeduplicator;
use super::similarity::TextSimilarity;
use crate::log_debug;

/// 迁移结果统计
#[derive(Debug, Clone, Default)]
pub struct MigrationResult {
    /// 是否执行了迁移
    pub migrated: bool,
    /// 从 MD 文件读取的条目数
    pub md_entries_count: usize,
    /// 去重后的条目数
    pub deduped_entries_count: usize,
    /// 移除的重复条目数
    pub removed_duplicates: usize,
    /// 备份的文件列表
    pub backed_up_files: Vec<String>,
}

/// 记忆格式迁移器
pub struct MemoryMigrator;

impl MemoryMigrator {
    /// 旧版 MD 文件名列表
    const OLD_MD_FILES: [(&'static str, MemoryCategory); 4] = [
        ("rules.md", MemoryCategory::Rule),
        ("preferences.md", MemoryCategory::Preference),
        ("patterns.md", MemoryCategory::Pattern),
        ("context.md", MemoryCategory::Context),
    ];

    /// 新版存储文件名
    pub const STORE_FILE: &'static str = "memories.json";

    /// 备份目录名
    const BACKUP_DIR: &'static str = "backup";

    /// 检查是否需要迁移
    ///
    /// 如果存在旧版 MD 文件且不存在新版 JSON 文件，则需要迁移
    pub fn needs_migration(memory_dir: &Path) -> bool {
        let store_path = memory_dir.join(Self::STORE_FILE);

        // 如果已存在新版文件，不需要迁移
        if store_path.exists() {
            return false;
        }

        // 检查是否存在任何旧版 MD 文件
        for (filename, _) in &Self::OLD_MD_FILES {
            let md_path = memory_dir.join(filename);
            if md_path.exists() {
                return true;
            }
        }

        false
    }

    /// 执行迁移
    ///
    /// 1. 读取所有旧版 MD 文件
    /// 2. 解析为 MemoryEntry
    /// 3. 执行去重
    /// 4. 写入新版 JSON 文件
    /// 5. 备份旧文件
    pub fn migrate(memory_dir: &Path, project_path: &str) -> Result<MigrationResult> {
        let mut result = MigrationResult::default();

        if !Self::needs_migration(memory_dir) {
            log_debug!("无需迁移：已存在 memories.json 或没有旧版 MD 文件");
            return Ok(result);
        }

        log_debug!("开始迁移记忆格式...");

        // 1. 读取所有旧版 MD 文件
        let mut all_entries: Vec<MemoryEntry> = Vec::new();

        for (filename, category) in &Self::OLD_MD_FILES {
            let md_path = memory_dir.join(filename);
            if md_path.exists() {
                let entries = Self::parse_md_file(&md_path, *category)?;
                all_entries.extend(entries);
            }
        }

        result.md_entries_count = all_entries.len();
        log_debug!("从 MD 文件读取了 {} 条记忆", result.md_entries_count);

        // 2. 执行去重
        let dedup = MemoryDeduplicator::default();
        let (deduped_entries, dedup_stats) = dedup.deduplicate(all_entries);

        result.deduped_entries_count = deduped_entries.len();
        result.removed_duplicates = dedup_stats.removed_count;
        log_debug!(
            "去重完成：移除 {} 条重复，保留 {} 条",
            result.removed_duplicates,
            result.deduped_entries_count
        );

        // 3. 创建新版存储结构
        let store = MemoryStore {
            version: "2.0".to_string(),
            project_path: project_path.to_string(),
            entries: deduped_entries,
            last_dedup_at: Utc::now(),
            config: MemoryConfig::default(),
        };

        // 4. 写入新版 JSON 文件
        let store_path = memory_dir.join(Self::STORE_FILE);
        let json = serde_json::to_string_pretty(&store)?;
        fs::write(&store_path, json)?;
        log_debug!("已写入新版存储文件: {}", store_path.display());

        // 5. 备份旧文件
        let backup_dir = memory_dir.join(Self::BACKUP_DIR);
        fs::create_dir_all(&backup_dir)?;

        for (filename, _) in &Self::OLD_MD_FILES {
            let md_path = memory_dir.join(filename);
            if md_path.exists() {
                let backup_path = backup_dir.join(filename);
                fs::rename(&md_path, &backup_path)?;
                result.backed_up_files.push(filename.to_string());
                log_debug!("已备份: {} -> {}", filename, backup_path.display());
            }
        }

        // 同时备份旧版 metadata.json（如果存在）
        let old_metadata = memory_dir.join("metadata.json");
        if old_metadata.exists() {
            let backup_metadata = backup_dir.join("metadata.json.bak");
            fs::rename(&old_metadata, &backup_metadata)?;
            result.backed_up_files.push("metadata.json".to_string());
        }

        result.migrated = true;
        log_debug!("迁移完成！");

        Ok(result)
    }

    /// 解析旧版 MD 文件
    fn parse_md_file(path: &Path, category: MemoryCategory) -> Result<Vec<MemoryEntry>> {
        let content = fs::read_to_string(path)?;
        let mut entries = Vec::new();

        // 按列表项解析，每个 "- " 开头的行是一个记忆条目
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("- ") && line.len() > 2 {
                let content = line[2..].trim(); // 去掉 "- " 前缀
                if !content.is_empty() {
                    let entry = MemoryEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        content: content.to_string(),
                        content_normalized: TextSimilarity::normalize(content),
                        category,
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                    };
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }

    /// 获取备份目录路径
    pub fn backup_dir(memory_dir: &Path) -> PathBuf {
        memory_dir.join(Self::BACKUP_DIR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_needs_migration() {
        let temp_dir = TempDir::new().unwrap();
        let memory_dir = temp_dir.path();

        // 空目录不需要迁移
        assert!(!MemoryMigrator::needs_migration(memory_dir));

        // 创建旧版 MD 文件
        fs::write(memory_dir.join("rules.md"), "# 规则\n- 规则1\n- 规则2\n").unwrap();
        assert!(MemoryMigrator::needs_migration(memory_dir));

        // 创建新版 JSON 文件后不需要迁移
        fs::write(memory_dir.join("memories.json"), "{}").unwrap();
        assert!(!MemoryMigrator::needs_migration(memory_dir));
    }

    #[test]
    fn test_parse_md_file() {
        let temp_dir = TempDir::new().unwrap();
        let md_path = temp_dir.path().join("test.md");

        let content = r#"# 测试规则

- 规则一
- 规则二
- 规则三

这是一些说明文字，不会被解析。
"#;
        fs::write(&md_path, content).unwrap();

        let entries = MemoryMigrator::parse_md_file(&md_path, MemoryCategory::Rule).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].content, "规则一");
        assert_eq!(entries[1].content, "规则二");
        assert_eq!(entries[2].content, "规则三");
    }
}
