use anyhow::{anyhow, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::HashSet;
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
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with("//?/") {
        normalized.drain(..4);
    }
    normalized
}

/// 仅折叠由非 Git 工作区父根明确覆盖的子项目，避免父子 watcher 重复监听。
pub(crate) fn collapse_workspace_watch_roots(
    roots: Vec<String>,
    user_excludes: &[String],
) -> Vec<String> {
    let mut normalized_roots = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let trimmed = root.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = PathBuf::from(trimmed);
        let canonical = path.canonicalize().unwrap_or(path);
        let normalized = normalize_path(&canonical);
        if seen.insert(path_key(&normalized)) {
            normalized_roots.push(normalized);
        }
    }

    let mut covered_children = HashSet::new();
    for root in &normalized_roots {
        let Ok(layout) = resolve_workspace(Path::new(root), user_excludes) else {
            continue;
        };
        if !layout.is_workspace {
            continue;
        }
        covered_children.extend(
            layout
                .projects
                .iter()
                .map(|project| path_key(&normalize_path(&project.root))),
        );
    }

    normalized_roots.retain(|root| !covered_children.contains(&path_key(root)));
    normalized_roots.sort_by_key(|root| path_key(root));
    normalized_roots
}

pub(crate) fn workspace_watch_covering_root(
    roots: &[String],
    project_root: &str,
    user_excludes: &[String],
) -> Option<String> {
    let project_path = PathBuf::from(project_root);
    let project_path = project_path.canonicalize().unwrap_or(project_path);
    let project_key = path_key(&normalize_path(&project_path));
    for root in roots {
        let Ok(layout) = resolve_workspace(Path::new(root), user_excludes) else {
            continue;
        };
        if layout.is_workspace
            && layout
                .projects
                .iter()
                .any(|project| path_key(&normalize_path(&project.root)) == project_key)
        {
            return Some(normalize_path(&layout.root));
        }
    }
    None
}

pub(crate) fn workspace_watch_scope_roots(
    project_root: &str,
    user_excludes: &[String],
) -> Vec<String> {
    let path = PathBuf::from(project_root);
    let path = path.canonicalize().unwrap_or(path);
    let normalized_root = normalize_path(&path);
    let Ok(layout) = resolve_workspace(&path, user_excludes) else {
        return vec![normalized_root];
    };
    if !layout.is_workspace {
        return vec![normalized_root];
    }
    let mut roots = vec![normalize_path(&layout.root)];
    roots.extend(
        layout
            .projects
            .iter()
            .map(|project| normalize_path(&project.root)),
    );
    roots
}

fn path_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_ascii_lowercase()
    } else {
        path.to_string()
    }
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

    #[test]
    fn workspace_watch_roots_keep_only_the_routing_parent() {
        let temp = tempdir().expect("应创建临时目录");
        let mut roots = Vec::new();
        for project in ["admin-ui", "debt-business", "server"] {
            let root = temp.path().join(project);
            fs::create_dir_all(root.join(".git")).expect("应创建子项目 Git 标记");
            roots.push(normalize_path(&root));
        }
        roots.push(normalize_path(temp.path()));

        let collapsed = collapse_workspace_watch_roots(roots, &[]);

        assert_eq!(collapsed, vec![normalize_path(temp.path())]);
        assert_eq!(
            workspace_watch_covering_root(
                &[normalize_path(temp.path())],
                &normalize_path(&temp.path().join("server")),
                &[],
            ),
            Some(normalize_path(temp.path()))
        );
        assert_eq!(
            workspace_watch_scope_roots(&normalize_path(temp.path()), &[]).len(),
            4
        );
    }

    #[test]
    fn git_parent_does_not_hide_an_independent_nested_watch_root() {
        let temp = tempdir().expect("应创建临时目录");
        fs::create_dir_all(temp.path().join(".git")).expect("应创建父项目 Git 标记");
        let nested = temp.path().join("vendor/nested");
        fs::create_dir_all(nested.join(".git")).expect("应创建嵌套项目 Git 标记");

        let collapsed = collapse_workspace_watch_roots(
            vec![normalize_path(temp.path()), normalize_path(&nested)],
            &[],
        );

        assert_eq!(collapsed.len(), 2);
        assert!(collapsed.contains(&normalize_path(temp.path())));
        assert!(collapsed.contains(&normalize_path(&nested)));
    }
}
