use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::types::{ProjectScopeRisk, ProjectScopeRiskLevel};
use super::watcher::normalize_project_path;

pub(crate) const MAX_SCANNED_ENTRIES: usize = 250_000;
pub(crate) const MAX_CANDIDATE_FILES: usize = 50_000;
pub(crate) const MAX_CANDIDATE_BYTES: u64 = 1024 * 1024 * 1024;

/// ACE 收集与文件监听共用的生成目录和缓存目录排除项。
pub(crate) const BUILTIN_EXCLUDE_PATTERNS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    "out",
    "coverage",
    ".next",
    ".nuxt",
    ".vite",
    ".cache",
    ".turbo",
    ".idea",
    ".vscode",
    ".gradle",
    ".mvn",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "__pycache__",
    "venv",
    ".venv",
    "env",
    "logs",
    "log",
    "tmp",
    ".tmp",
    "*.log",
    "*.tmp",
    "*.swp",
    "*.swo",
    "*.pyc",
    "*.class",
    ".DS_Store",
];

const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "pyproject.toml",
    "CMakeLists.txt",
];

#[derive(Debug, Clone)]
pub(crate) struct ProjectScopeAssessment {
    pub scanned_entries: usize,
    pub candidate_files: usize,
    pub candidate_bytes: u64,
    pub project_markers: Vec<String>,
}

impl ProjectScopeAssessment {
    fn into_scale_risk(self, reason_code: &str, reason: String) -> ProjectScopeRisk {
        ProjectScopeRisk {
            level: ProjectScopeRiskLevel::ExcessiveScale,
            reason_code: reason_code.to_string(),
            reason,
            scanned_entries: self.scanned_entries,
            candidate_files: self.candidate_files,
            candidate_bytes: self.candidate_bytes,
            project_markers: self.project_markers,
            requires_secondary_confirmation: false,
            detected_at: chrono::Utc::now(),
        }
    }
}

/// 用户配置只能增加排除项，不能移除保护生成目录的内置规则。
pub(crate) fn effective_exclude_patterns(user_patterns: Option<&[String]>) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for pattern in user_patterns
        .into_iter()
        .flatten()
        .map(String::as_str)
        .chain(BUILTIN_EXCLUDE_PATTERNS.iter().copied())
    {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            result.push(trimmed.to_string());
        }
    }
    result
}

pub(crate) fn normalize_root(path: &str) -> String {
    normalize_project_path(
        &PathBuf::from(path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(path))
            .to_string_lossy(),
    )
}

pub(crate) fn is_confirmed_project_root(project_root: &str) -> bool {
    let normalized_root = normalize_root(project_root);
    crate::config::load_standalone_config()
        .ok()
        .and_then(|config| config.mcp_config.acemcp_confirmed_project_roots)
        .unwrap_or_default()
        .into_iter()
        .any(|path| paths_equal(&normalize_root(&path), &normalized_root))
}

pub(crate) fn critical_path_risk(project_root: &str) -> Option<ProjectScopeRisk> {
    let normalized_root = normalize_root(project_root);
    let root_path = PathBuf::from(&normalized_root);
    let is_filesystem_root = root_path.parent().is_none();
    let is_home = dirs::home_dir()
        .map(|home| paths_equal(&normalize_root(&home.to_string_lossy()), &normalized_root))
        .unwrap_or(false);

    let (reason_code, reason) = if is_filesystem_root {
        (
            "filesystem_root",
            "所选路径是文件系统根目录，索引范围可能覆盖整块磁盘。",
        )
    } else if is_home {
        (
            "user_home",
            "所选路径是当前用户主目录，索引范围可能覆盖大量非项目文件。",
        )
    } else {
        return None;
    };

    Some(ProjectScopeRisk {
        level: ProjectScopeRiskLevel::CriticalPath,
        reason_code: reason_code.to_string(),
        reason: reason.to_string(),
        scanned_entries: 0,
        candidate_files: 0,
        candidate_bytes: 0,
        project_markers: detect_root_markers(&root_path),
        requires_secondary_confirmation: true,
        detected_at: chrono::Utc::now(),
    })
}

pub(crate) fn preflight_project_scope(
    project_root: &str,
    text_extensions: &[String],
    exclude_patterns: &[String],
) -> Result<Option<ProjectScopeRisk>> {
    preflight_with_limits(
        project_root,
        text_extensions,
        exclude_patterns,
        MAX_SCANNED_ENTRIES,
        MAX_CANDIDATE_FILES,
        MAX_CANDIDATE_BYTES,
    )
}

fn preflight_with_limits(
    project_root: &str,
    text_extensions: &[String],
    exclude_patterns: &[String],
    max_entries: usize,
    max_files: usize,
    max_bytes: u64,
) -> Result<Option<ProjectScopeRisk>> {
    let root = PathBuf::from(project_root)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(project_root));
    if !root.exists() || !root.is_dir() {
        anyhow::bail!("项目根目录不存在: {}", project_root);
    }

    let exclude_globset = super::mcp::build_exclude_globset(exclude_patterns)?;
    let gitignore = build_gitignore(&root);
    let mut stack = vec![root.clone()];
    let mut assessment = ProjectScopeAssessment {
        scanned_entries: 0,
        candidate_files: 0,
        candidate_bytes: 0,
        project_markers: detect_root_markers(&root),
    };

    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            assessment.scanned_entries = assessment.scanned_entries.saturating_add(1);
            if assessment.scanned_entries >= max_entries {
                return Ok(Some(assessment.into_scale_risk(
                    "scanned_entries_limit",
                    format!(
                        "预检目录项达到安全上限 {}，已在读取文件内容前暂停索引。",
                        max_entries
                    ),
                )));
            }

            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // 中文说明：不跟随符号链接或目录联接，避免越出所选项目范围或形成循环。
            if file_type.is_symlink() {
                continue;
            }
            if gitignore.as_ref().is_some_and(|rules| {
                rules
                    .matched_path_or_any_parents(&path, file_type.is_dir())
                    .is_ignore()
            }) || super::mcp::should_exclude(&path, &root, Some(&exclude_globset))
            {
                continue;
            }
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() || !has_text_extension(&path, text_extensions) {
                continue;
            }

            assessment.candidate_files = assessment.candidate_files.saturating_add(1);
            assessment.candidate_bytes = assessment
                .candidate_bytes
                .saturating_add(entry.metadata().map(|value| value.len()).unwrap_or(0));
            if assessment.candidate_files >= max_files {
                return Ok(Some(assessment.into_scale_risk(
                    "candidate_files_limit",
                    format!(
                        "候选源码/文本文件达到安全上限 {}，已在读取文件内容前暂停索引。",
                        max_files
                    ),
                )));
            }
            if assessment.candidate_bytes >= max_bytes {
                return Ok(Some(assessment.into_scale_risk(
                    "candidate_bytes_limit",
                    format!(
                        "候选文件总大小达到安全上限 {} 字节，已在读取文件内容前暂停索引。",
                        max_bytes
                    ),
                )));
            }
        }
    }

    Ok(None)
}

fn build_gitignore(root: &Path) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    let path = root.join(".gitignore");
    if !path.exists() || builder.add(path).is_some() {
        return None;
    }
    builder.build().ok()
}

fn has_text_extension(path: &Path, text_extensions: &[String]) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            let extension = format!(".{}", extension);
            text_extensions
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&extension))
        })
        .unwrap_or(false)
}

fn detect_root_markers(root: &Path) -> Vec<String> {
    let mut markers = PROJECT_MARKERS
        .iter()
        .filter(|marker| root.join(marker).exists())
        .map(|marker| (*marker).to_string())
        .collect::<Vec<_>>();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".sln") || name.ends_with(".csproj") {
                markers.push(name);
            }
        }
    }
    markers.sort();
    markers.dedup();
    markers
}

fn paths_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_excludes_keep_builtins_and_deduplicate_user_values() {
        let user = vec!["node_modules".to_string(), "custom-cache".to_string()];
        let result = effective_exclude_patterns(Some(&user));
        assert_eq!(
            result
                .iter()
                .filter(|value| *value == "node_modules")
                .count(),
            1
        );
        assert!(result.iter().any(|value| value == "custom-cache"));
        assert!(result.iter().any(|value| value == "target"));
    }

    #[test]
    fn current_user_home_is_a_critical_path() {
        let home = dirs::home_dir().expect("测试环境应提供用户主目录");
        let risk = critical_path_risk(&home.to_string_lossy()).expect("用户主目录必须被拦截");
        assert_eq!(risk.level, ProjectScopeRiskLevel::CriticalPath);
        assert_eq!(risk.reason_code, "user_home");
        assert!(risk.requires_secondary_confirmation);
    }

    #[test]
    fn preflight_stops_at_candidate_file_limit_without_reading_contents() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(temp.path().join("one.rs"), "fn one() {}").unwrap();
        fs::write(temp.path().join("two.rs"), "fn two() {}").unwrap();

        let risk = preflight_with_limits(
            &temp.path().to_string_lossy(),
            &[".rs".to_string()],
            &effective_exclude_patterns(None),
            100,
            2,
            u64::MAX,
        )
        .unwrap()
        .unwrap();

        assert_eq!(risk.reason_code, "candidate_files_limit");
        assert!(risk
            .project_markers
            .iter()
            .any(|value| value == "Cargo.toml"));
    }

    #[test]
    fn preflight_ignores_generated_directories() {
        let temp = tempfile::tempdir().unwrap();
        let generated = temp.path().join("node_modules");
        fs::create_dir_all(&generated).unwrap();
        fs::write(generated.join("large.ts"), "generated").unwrap();
        fs::write(temp.path().join("main.ts"), "export {}").unwrap();

        let risk = preflight_with_limits(
            &temp.path().to_string_lossy(),
            &[".ts".to_string()],
            &effective_exclude_patterns(None),
            100,
            2,
            u64::MAX,
        )
        .unwrap();

        assert!(risk.is_none());
    }
}
