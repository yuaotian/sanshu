//! 记忆历史清理模块
//!
//! 提供只预览、不改写正文的本地相似度清理能力。

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::similarity::TextSimilarity;
use super::types::{MemoryCategory, MemoryEntry};

fn default_cleanup_threshold() -> f64 {
    0.55
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CleanupPreviewRequest {
    /// 本地相似度阈值，达到后进入候选清理组
    #[serde(default = "default_cleanup_threshold")]
    pub threshold: f64,
    /// 需要参与清理的分类；空列表表示全部分类
    #[serde(default)]
    pub categories: Vec<String>,
    /// 是否允许跨分类合并；默认关闭以降低误删风险
    #[serde(default)]
    pub include_cross_category: bool,
}

impl Default for CleanupPreviewRequest {
    fn default() -> Self {
        Self {
            threshold: default_cleanup_threshold(),
            categories: Vec::new(),
            include_cross_category: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CleanupGroupEntry {
    pub id: String,
    pub content: String,
    pub category: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&MemoryEntry> for CleanupGroupEntry {
    fn from(entry: &MemoryEntry) -> Self {
        Self {
            id: entry.id.clone(),
            content: entry.content.clone(),
            category: entry.category.display_name().to_string(),
            created_at: entry.created_at.to_rfc3339(),
            updated_at: entry.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CleanupGroup {
    pub group_id: String,
    pub category: String,
    pub max_similarity: f64,
    pub recommended_keep_id: String,
    pub default_delete_ids: Vec<String>,
    pub entries: Vec<CleanupGroupEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CleanupPreviewResult {
    pub original_count: usize,
    pub candidate_group_count: usize,
    pub estimated_removed_count: usize,
    pub groups: Vec<CleanupGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CleanupApplyGroup {
    pub group_id: String,
    pub keep_id: String,
    pub delete_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CleanupApplyRequest {
    #[serde(default)]
    pub auto_backup: bool,
    #[serde(default)]
    pub groups: Vec<CleanupApplyGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CleanupApplyResult {
    pub backup_file: Option<String>,
    pub removed_count: usize,
    pub remaining_count: usize,
    pub removed_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackupInfo {
    pub file_name: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestoreBackupResult {
    pub restored_file: String,
    pub safety_backup_file: Option<String>,
    pub entry_count: usize,
}

#[derive(Debug, Clone)]
struct CandidateEntry<'a> {
    entry: &'a MemoryEntry,
}

#[derive(Debug, Clone)]
struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
        }
    }

    fn find(&mut self, idx: usize) -> usize {
        if self.parent[idx] != idx {
            let root = self.find(self.parent[idx]);
            self.parent[idx] = root;
        }
        self.parent[idx]
    }

    fn union(&mut self, a: usize, b: usize) {
        let root_a = self.find(a);
        let root_b = self.find(b);
        if root_a != root_b {
            self.parent[root_b] = root_a;
        }
    }
}

/// 生成历史清理预览，只返回候选组，不修改任何记忆。
pub fn preview_cleanup(
    entries: &[MemoryEntry],
    request: &CleanupPreviewRequest,
) -> CleanupPreviewResult {
    let threshold = request.threshold.clamp(0.0, 1.0);
    let category_filter = build_category_filter(&request.categories);
    let candidates: Vec<CandidateEntry<'_>> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            category_filter
                .as_ref()
                .map(|set| set.contains(&entry.category))
                .unwrap_or(true)
        })
        .map(|(_, entry)| CandidateEntry { entry })
        .collect();

    if candidates.len() < 2 {
        return CleanupPreviewResult {
            original_count: entries.len(),
            ..CleanupPreviewResult::default()
        };
    }

    let mut dsu = DisjointSet::new(candidates.len());
    let mut pair_scores: HashMap<(usize, usize), f64> = HashMap::new();

    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            if !request.include_cross_category
                && candidates[i].entry.category != candidates[j].entry.category
            {
                continue;
            }

            let similarity = TextSimilarity::calculate_enhanced(
                &candidates[i].entry.content,
                &candidates[j].entry.content,
            );
            if similarity >= threshold {
                dsu.union(i, j);
                pair_scores.insert((i, j), similarity);
            }
        }
    }

    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for idx in 0..candidates.len() {
        let root = dsu.find(idx);
        components.entry(root).or_default().push(idx);
    }

    let mut groups = Vec::new();
    for component in components.values().filter(|items| items.len() > 1) {
        let max_similarity = component_max_similarity(component, &pair_scores);
        let keep_idx = recommend_keep_index(component, &candidates);
        let recommended_keep_id = candidates[keep_idx].entry.id.clone();
        let mut cleanup_entries: Vec<CleanupGroupEntry> = component
            .iter()
            .map(|idx| CleanupGroupEntry::from(candidates[*idx].entry))
            .collect();
        cleanup_entries.sort_by(|a, b| {
            let a_keep = a.id == recommended_keep_id;
            let b_keep = b.id == recommended_keep_id;
            b_keep.cmp(&a_keep).then_with(|| a.id.cmp(&b.id))
        });

        let default_delete_ids = cleanup_entries
            .iter()
            .filter(|entry| entry.id != recommended_keep_id)
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let category = if request.include_cross_category {
            "混合".to_string()
        } else {
            candidates[component[0]]
                .entry
                .category
                .display_name()
                .to_string()
        };

        groups.push(CleanupGroup {
            group_id: format!("cleanup-{}", groups.len() + 1),
            category,
            max_similarity,
            recommended_keep_id,
            default_delete_ids,
            entries: cleanup_entries,
        });
    }

    groups.sort_by(|a, b| {
        b.max_similarity
            .partial_cmp(&a.max_similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let estimated_removed_count = groups.iter().map(|g| g.default_delete_ids.len()).sum();
    CleanupPreviewResult {
        original_count: entries.len(),
        candidate_group_count: groups.len(),
        estimated_removed_count,
        groups,
    }
}

fn build_category_filter(categories: &[String]) -> Option<HashSet<MemoryCategory>> {
    if categories.is_empty() {
        return None;
    }

    Some(
        categories
            .iter()
            .map(|category| MemoryCategory::from_str(category))
            .collect(),
    )
}

fn component_max_similarity(
    component: &[usize],
    pair_scores: &HashMap<(usize, usize), f64>,
) -> f64 {
    let mut max_similarity = 0.0;
    for i in 0..component.len() {
        for j in (i + 1)..component.len() {
            let a = component[i].min(component[j]);
            let b = component[i].max(component[j]);
            if let Some(score) = pair_scores.get(&(a, b)) {
                if *score > max_similarity {
                    max_similarity = *score;
                }
            }
        }
    }
    max_similarity
}

fn recommend_keep_index(component: &[usize], candidates: &[CandidateEntry<'_>]) -> usize {
    *component
        .iter()
        .max_by(|a, b| compare_keep_candidate(candidates[**a].entry, candidates[**b].entry))
        .expect("component 至少包含一个候选")
}

fn compare_keep_candidate(a: &MemoryEntry, b: &MemoryEntry) -> std::cmp::Ordering {
    a.updated_at
        .cmp(&b.updated_at)
        .then_with(|| a.content.len().cmp(&b.content.len()))
        .then_with(|| b.id.cmp(&a.id))
}

pub fn parse_backup_timestamp(file_name: &str) -> Option<DateTime<Utc>> {
    let stamp = file_name.strip_suffix(".memories.json")?;
    let (base, millis) = stamp.rsplit_once('-')?;
    let millis = millis.parse::<i64>().ok()?;
    chrono::NaiveDateTime::parse_from_str(base, "%Y%m%d-%H%M%S")
        .ok()
        .map(|dt| {
            DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc)
                + chrono::Duration::milliseconds(millis)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
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
    fn preview_cleanup_groups_same_category_duplicates() {
        let entries = vec![
            entry("a", "不要编译，用户自己编译", MemoryCategory::Rule, 0),
            entry("b", "禁止编译，用户自己编译", MemoryCategory::Rule, 10),
            entry("c", "配置数据库连接池大小", MemoryCategory::Rule, 0),
        ];

        let result = preview_cleanup(
            &entries,
            &CleanupPreviewRequest {
                threshold: 0.55,
                categories: vec!["规范".to_string()],
                include_cross_category: false,
            },
        );

        assert_eq!(result.candidate_group_count, 1);
        assert_eq!(result.estimated_removed_count, 1);
        assert_eq!(result.groups[0].recommended_keep_id, "b");
    }

    #[test]
    fn preview_cleanup_does_not_cross_category_by_default() {
        let entries = vec![
            entry("a", "不要编译，用户自己编译", MemoryCategory::Rule, 0),
            entry(
                "b",
                "禁止编译，用户自己编译",
                MemoryCategory::Preference,
                10,
            ),
        ];

        let result = preview_cleanup(&entries, &CleanupPreviewRequest::default());

        assert_eq!(result.candidate_group_count, 0);
        assert_eq!(result.estimated_removed_count, 0);
    }
}
