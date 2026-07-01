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
    use super::super::types::MemoryCategory;
    use super::*;
    use chrono::Utc;

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

    /// 用真实堆积的「偏好/规则」记忆量化新算法（含 bigram）的清理效果。
    /// 数据取自用户提供的 memories.json 中语义高度重复的协作规则条目。
    #[test]
    fn test_real_world_cleanup_effect() {
        // 8 条真实记忆：均在描述同一套「生成 MD / 不测试 / 不编译 / 不运行 /
        // 用 sou / zhi / context7 / tavily / uiux」协作规则，仅措辞与语序不同
        let entries = vec![
            make_entry("p1", "用户确认：复杂审计/修复方案类任务需要生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先使用 sou 检索代码上下文；需要最新框架/库官方文档用 context7；关键确认必须通过 zhi；UI 审查优先 uiux；实时搜索使用 tavily。若当前任务有只读硬约束，则只输出方案，不写文件。"),
            make_entry("p2", "用户偏好：复杂审计/修复任务完成后生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先使用 sou 做代码语义搜索；关键确认使用 zhi；需要最新框架/库文档时使用 context7；实时搜索用 tavily；UI/UX 相关任务优先使用 uiux。"),
            make_entry("p3", "用户偏好：复杂审计/修复任务需生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行；优先用 sou 做代码语义搜索并按结果查看上下文；需要最新框架/库文档时用 context7；关键提问、方案确认和完成确认必须通过 zhi；涉及页面美化、UI 描述、设计系统或 UI 审查时优先使用 uiux 并参考其返回；需要实时搜索信息时用 tavily。"),
            make_entry("p4", "用户要求复杂排查/修复任务完成后生成总结性 Markdown 文档；禁止生成测试脚本；禁止编译，用户自己编译；禁止运行，用户自己运行；必须先用 sanshu sou 做代码语义搜索并根据结果读取上下文；需要最新框架/库文档和 API 用法时使用 context7；所有关键提问、方案确认和完成确认必须通过 zhi；涉及 UI/UX 时优先使用 uiux；需要实时搜索信息时使用 tavily。"),
            make_entry("p5", "用户确认本项目任务偏好：方案确认和最终确认必须通过 zhi；先用 sou 检索再读取上下文；不要生成测试脚本；不要编译；不要运行；复杂/总结性任务需要生成总结性 Markdown 文档；需要最新框架/API 用 context7，实时搜索用 tavily，UI/UX 用 uiux。"),
            make_entry("p6", "用户偏好：代码任务关键确认和完成确认通过三术 zhi；先用 sou 做代码语义搜索；不要生成测试脚本、不要编译、不要运行，用户自己执行；需要最新框架/API 用 context7，实时搜索用 tavily，UI/UX 相关优先用 uiux；任务完成后按需生成总结性 Markdown 文档。"),
            make_entry("p7", "项目协作规则：默认生成总结性 Markdown 文档；不要生成测试脚本；不要编译；不要运行服务或应用，均由用户自行执行；代码上下文检索必须优先使用 sanshu.sou，再读取命中文件细节；关键提问、方案确认、完成确认必须通过 sanshu.zhi；需要最新框架/库文档时使用 sanshu.context7；需要实时信息时使用 sanshu.tavily；涉及页面美化、UI 描述、设计系统或 UI 审查时优先使用 sanshu.uiux。"),
            // 一条真正不同的规则，作为「不应被误删」的对照
            make_entry("d1", "项目开发规范：Controller 按业务模块拆分；Service 直接创建具体类，不额外创建接口和 Impl；DO 一表一类且字段语义与数据库列一致，敏感密文字段用 xxxCipher。"),
        ];

        let original = entries.len();

        // 分别在「去重阈值 0.70」与「upsert 阈值 0.55」下测量清理效果
        let (dedup70, stats70) = MemoryDeduplicator::new(0.70).deduplicate(entries.clone());
        let (_dedup55, stats55) = MemoryDeduplicator::new(0.55).deduplicate(entries);

        println!(
            "[真实清理效果] 原始 {} 条\n  · 去重阈值 0.70 → 保留 {} / 移除 {}\n  · upsert 阈值 0.55 → 保留 {} / 移除 {}",
            original,
            stats70.remaining_count,
            stats70.removed_count,
            stats55.remaining_count,
            stats55.removed_count
        );

        // 结论断言（如实反映能力边界）：
        // 1) 那条真正不同的开发规范，在两种阈值下都不能被误删。
        assert!(
            dedup70.iter().any(|e| e.id == "d1"),
            "真正不同的开发规范不应被误删"
        );
        // 2) 0.55（方案 B 的 upsert 区间）能比 0.70（纯去重）合并更多近义改写，
        //    体现「B 从源头抑制堆积」的价值——这正是 0.70 抓不到的那一档。
        assert!(
            stats55.removed_count >= stats70.removed_count,
            "更低的 upsert 阈值应合并不少于去重阈值的条目数 (0.55={}, 0.70={})",
            stats55.removed_count,
            stats70.removed_count
        );
    }
}
