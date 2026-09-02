use anyhow::{anyhow, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::fs;
use std::path::{Path, PathBuf};

use super::acemcp::scope_guard::{effective_exclude_patterns, MAX_SCANNED_ENTRIES};

pub(crate) const MAX_WORKSPACE_DEPTH: usize = 4;
pub(crate) const MAX_WORKSPACE_PROJECTS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceProject {
    pub root: PathBuf,
    pub relative_path: String,
}

impl WorkspaceProject {
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&self.relative_path)
            .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceLayout {
    pub root: PathBuf,
    pub is_workspace: bool,
    pub projects: Vec<WorkspaceProject>,
}

impl WorkspaceLayout {
    pub fn project_excludes(&self) -> Vec<String> {
        if !self.is_workspace {
            return Vec::new();
        }
        self.projects
            .iter()
            .map(|project| project.relative_path.clone())
            .collect()
    }
}

pub(crate) fn resolve_workspace(root: &Path, user_excludes: &[String]) -> Result<WorkspaceLayout> {
    let root = root
        .canonicalize()
        .with_context(|| format!("工作区路径无效: {}", root.display()))?;
    if !root.is_dir() {
        return Err(anyhow!("工作区路径不是目录: {}", root.display()));
    }

    if is_git_root(&root) {
        return Ok(single_project_layout(root));
    }

    let exclude_patterns = effective_exclude_patterns(Some(user_excludes));
    let excludes = build_exclude_globset(&exclude_patterns)?;
    let mut stack = vec![(root.clone(), 0usize)];
    let mut projects = Vec::new();
    let mut scanned_entries = 0usize;

    while let Some((directory, depth)) = stack.pop() {
        if depth >= MAX_WORKSPACE_DEPTH {
            continue;
        }
        let mut entries = match fs::read_dir(&directory) {
            Ok(entries) => entries.flatten().collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

        for entry in entries {
            scanned_entries = scanned_entries.saturating_add(1);
            if scanned_entries > MAX_SCANNED_ENTRIES {
                return Err(anyhow!(
                    "工作区发现目录项超过安全上限 {}",
                    MAX_SCANNED_ENTRIES
                ));
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            let path = entry.path();
            if is_excluded(&root, &path, &excludes) {
                continue;
            }
            if is_git_root(&path) {
                projects.push(WorkspaceProject {
                    relative_path: relative_path(&root, &path)?,
                    root: path,
                });
                if projects.len() > MAX_WORKSPACE_PROJECTS {
                    return Err(anyhow!(
                        "工作区独立 Git 项目超过安全上限 {}",
                        MAX_WORKSPACE_PROJECTS
                    ));
                }
                continue;
            }
            stack.push((path, depth + 1));
        }
    }

    if projects.is_empty() {
        return Ok(single_project_layout(root));
    }
    projects.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(WorkspaceLayout {
        root,
        is_workspace: true,
        projects,
    })
}

pub(crate) fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn single_project_layout(root: PathBuf) -> WorkspaceLayout {
    WorkspaceLayout {
        projects: vec![WorkspaceProject {
            root: root.clone(),
            relative_path: String::new(),
        }],
        root,
        is_workspace: false,
    }
}

fn is_git_root(path: &Path) -> bool {
    path.join(".git").exists()
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    Ok(normalize_path(path.strip_prefix(root).with_context(
        || format!("项目路径不在工作区内: {}", path.display()),
    )?))
}

fn build_exclude_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        }
    }
    builder.build().context("构建工作区排除规则失败")
}

fn is_excluded(root: &Path, path: &Path, excludes: &GlobSet) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let normalized = normalize_path(relative);
    excludes.is_match(&normalized)
        || relative
            .iter()
            .filter_map(|part| part.to_str())
            .any(|part| excludes.is_match(part))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn git_root_remains_a_single_project() {
        let temp = tempdir().expect("应创建临时目录");
        fs::create_dir_all(temp.path().join(".git")).expect("应创建 Git 标记");
        fs::create_dir_all(temp.path().join("vendor/nested/.git")).expect("应创建嵌套仓库");

        let layout = resolve_workspace(temp.path(), &[]).expect("应解析单项目");

        assert!(!layout.is_workspace);
        assert_eq!(layout.projects.len(), 1);
        assert_eq!(layout.projects[0].root, temp.path().canonicalize().unwrap());
    }

    #[test]
    fn non_git_parent_discovers_independent_projects_and_stops_at_roots() {
        let temp = tempdir().expect("应创建临时目录");
        for project in ["admin-ui", "debt-business", "server"] {
            fs::create_dir_all(temp.path().join(project).join(".git"))
                .expect("应创建子项目 Git 标记");
        }
        fs::create_dir_all(temp.path().join("server/vendor/ignored/.git"))
            .expect("应创建仓库内部嵌套标记");

        let layout = resolve_workspace(temp.path(), &[]).expect("应解析工作区");

        assert!(layout.is_workspace);
        assert_eq!(
            layout
                .projects
                .iter()
                .map(|project| project.relative_path.as_str())
                .collect::<Vec<_>>(),
            vec!["admin-ui", "debt-business", "server"]
        );
        assert_eq!(layout.project_excludes().len(), 3);
    }

    #[test]
    fn git_file_is_treated_as_a_project_marker() {
        let temp = tempdir().expect("应创建临时目录");
        let project = temp.path().join("worktree");
        fs::create_dir_all(&project).expect("应创建 worktree");
        fs::write(project.join(".git"), "gitdir: ../meta").expect("应写入 Git 文件标记");

        let layout = resolve_workspace(temp.path(), &[]).expect("应解析 worktree 项目");

        assert!(layout.is_workspace);
        assert_eq!(layout.projects[0].relative_path, "worktree");
    }

    #[test]
    fn configured_excludes_remove_candidate_project_roots() {
        let temp = tempdir().expect("应创建临时目录");
        fs::create_dir_all(temp.path().join("included/.git")).expect("应创建项目");
        fs::create_dir_all(temp.path().join("ignored/.git")).expect("应创建排除项目");

        let layout =
            resolve_workspace(temp.path(), &["ignored".to_string()]).expect("应按排除规则解析");

        assert_eq!(layout.projects.len(), 1);
        assert_eq!(layout.projects[0].relative_path, "included");
    }
}
