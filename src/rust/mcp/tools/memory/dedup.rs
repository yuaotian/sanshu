//! 记忆去重模块
//!
//! 提供记忆条目的去重检测和批量去重功能

use super::similarity::TextSimilarity;
use super::types::MemoryEntry;

/// 去重检测结果
#[derive(Debug, Clone)]
pub struct DuplicateInfo {
    /// 是否为重复记忆
    pub is_duplicate: bool,
    /// 最高相似度
    pub similarity: f64,
    /// 匹配到的记忆条目 ID（如果有）
    pub matched_id: Option<String>,
    /// 匹配到的记忆内容（如果有）
    pub matched_content: Option<String>,
}

/// 去重统计结果
#[derive(Debug, Clone, Default)]
pub struct DedupResult {
    /// 原始条目数
    pub original_count: usize,
    /// 移除的条目数
    pub removed_count: usize,
    /// 保留的条目数
    pub remaining_count: usize,
    /// 被移除的条目 ID 列表
    pub removed_ids: Vec<String>,
}

/// 记忆去重器
pub struct MemoryDeduplicator {
    /// 相似度阈值（0.0 ~ 1.0）
    threshold: f64,
}

impl Default for MemoryDeduplicator {
    fn default() -> Self {
        Self::new(0.70) // 默认 70% 阈值
    }
}

impl MemoryDeduplicator {
    /// 创建去重器
    ///
    /// # 参数
    /// - `threshold`: 相似度阈值 (0.0 ~ 1.0)，超过此阈值视为重复
    pub fn new(threshold: f64) -> Self {
        Self {
            threshold: threshold.clamp(0.0, 1.0),
        }
    }

    /// 获取当前阈值
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// 检查新内容是否与已有记忆重复
    ///
    /// # 参数
    /// - `new_content`: 要检查的新内容
    /// - `existing`: 已有的记忆列表
    ///
    /// # 返回
    /// 去重检测结果
    pub fn check_duplicate(&self, new_content: &str, existing: &[MemoryEntry]) -> DuplicateInfo {
        let mut max_similarity = 0.0;
        let mut matched_id = None;
        let mut matched_content = None;

        for entry in existing {
            // 使用增强版算法，包含子串检测
            let similarity = TextSimilarity::calculate_enhanced(new_content, &entry.content);
            if similarity > max_similarity {
                max_similarity = similarity;
                if similarity >= self.threshold {
                    matched_id = Some(entry.id.clone());
                    matched_content = Some(entry.content.clone());
                }
            }
        }

        DuplicateInfo {
            is_duplicate: max_similarity >= self.threshold,
            similarity: max_similarity,
            matched_id,
            matched_content,
        }
    }

    /// 对记忆列表进行去重
    ///
    /// 保留先出现的记忆，移除后出现的重复记忆
    ///
    /// # 参数
    /// - `entries`: 记忆列表
    ///
    /// # 返回
    /// (去重后的列表, 去重统计结果)
    pub fn deduplicate(&self, entries: Vec<MemoryEntry>) -> (Vec<MemoryEntry>, DedupResult) {
        let original_count = entries.len();
        let mut result: Vec<MemoryEntry> = Vec::new();
        let mut removed_ids: Vec<String> = Vec::new();

        for entry in entries {
            let mut is_dup = false;

            for kept in &result {
                // 使用增强版算法，包含子串检测
                let similarity = TextSimilarity::calculate_enhanced(&entry.content, &kept.content);
                if similarity >= self.threshold {
                    is_dup = true;
                    break;
                }
            }

            if is_dup {
                removed_ids.push(entry.id.clone());
            } else {
                result.push(entry);
            }
        }

        let remaining_count = result.len();
        let removed_count = original_count - remaining_count;

        let stats = DedupResult {
            original_count,
            removed_count,
            remaining_count,
            removed_ids,
        };

        (result, stats)
    }

    /// 快速检查内容是否与现有列表中的任何内容相似
    ///
    /// 仅返回布尔值，适用于插入时的快速检查
    pub fn is_duplicate(&self, new_content: &str, existing: &[MemoryEntry]) -> bool {
        for entry in existing {
            let similarity = TextSimilarity::calculate_enhanced(new_content, &entry.content);
            if similarity >= self.threshold {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use super::super::types::MemoryCategory;

    fn make_entry(id: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            content_normalized: TextSimilarity::normalize(content),
            category: MemoryCategory::Rule,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_check_duplicate() {
        let dedup = MemoryDeduplicator::new(0.70);
        let existing = vec![
            make_entry("1", "使用 KISS 原则"),
            make_entry("2", "不要生成测试脚本"),
        ];

        // 相似记忆应该被检测为重复
        let result = dedup.check_duplicate("使用KISS原则", &existing);
        assert!(result.is_duplicate);
        assert_eq!(result.matched_id, Some("1".to_string()));

        // 不相关记忆应该通过
        let result = dedup.check_duplicate("配置数据库连接", &existing);
        assert!(!result.is_duplicate);
    }

    #[test]
    fn test_deduplicate() {
        let dedup = MemoryDeduplicator::new(0.70);
        let entries = vec![
            make_entry("1", "使用 KISS 原则"),
            make_entry("2", "使用KISS原则"),
            make_entry("3", "遵循 KISS 原则"),
            make_entry("4", "不要生成测试脚本"),
            make_entry("5", "后端使用 Rust"),
        ];

        let (deduped, stats) = dedup.deduplicate(entries);

        // 应该保留 3 条（1, 4, 5）
        assert_eq!(stats.original_count, 5);
        assert_eq!(stats.removed_count, 2);
        assert_eq!(deduped.len(), 3);
    }
}
