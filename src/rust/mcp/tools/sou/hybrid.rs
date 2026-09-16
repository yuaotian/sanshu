use super::query::QueryPlan;
use super::*;
use std::collections::BTreeMap;
use std::future::Future;

const DEFAULT_BUDGET_MS: u64 = 8000;

pub(super) async fn run(
    request: &SouRequest,
    config: &SouRuntimeConfig,
) -> Result<BackendRunResult, String> {
    if !config.local_enabled {
        return Err("hybrid 需要启用 Local 检索".to_string());
    }
    let started = Instant::now();
    let mut local = run_local(request, config).await?;
    let local_duration_ms = local.duration_ms;
    let plan = QueryPlan::new(&request.query, request.intent);
    let sections = parse_sou_sections(&local.text, BACKEND_LOCAL);
    let root = Path::new(&request.project_root_path)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    // 中文说明：工作区子项目也重新核对 ignore，防止本地专用结果混入远端增强输入。
    let local_only = local
        .retrieval
        .as_ref()
        .and_then(|meta| meta["local_only"].as_bool())
        .unwrap_or(false)
        || sections.iter().any(|section| {
            let (path, _) = split_location_range(&section.location);
            local::path_ignored(&root, Path::new(path))
        });
    let reason = enhancement_reason(&plan, &sections, local_only);
    let budget_ms = request
        .timeout_ms
        .unwrap_or(DEFAULT_BUDGET_MS)
        .clamp(100, 300000);
    let mut metadata = local
        .retrieval
        .take()
        .unwrap_or_else(|| serde_json::json!({}));
    metadata["local_only"] = local_only.into();
    metadata["local_duration_ms"] = local_duration_ms.into();
    metadata["enhancement_budget_ms"] = budget_ms.into();
    if let Err(skip_reason) = reason {
        metadata["enhancement_state"] = "skipped".into();
        metadata["enhancement_reason"] = skip_reason.into();
        local.retrieval = Some(metadata);
        local.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(local);
    }
    let reason = reason.unwrap();
    let seed = candidate_context(&root, &sections);
    let seed_chars = seed.chars().count();
    let mut remote_request = request.clone();
    remote_request.timeout_ms = Some(budget_ms);
    let remote_started = Instant::now();
    let remote = within_budget(
        Duration::from_millis(budget_ms),
        run_fast_context_seeded(&remote_request, &config.fast_context, false, Some(seed)),
    )
    .await;
    metadata["enhancement_reason"] = reason.into();
    metadata["seed_chars"] = seed_chars.into();
    metadata["enhancement_duration_ms"] = (remote_started.elapsed().as_millis() as u64).into();
    match remote {
        Ok(fast) => {
            let remote_sections = parse_sou_sections(&fast.text, BACKEND_FAST_CONTEXT);
            if remote_sections.is_empty() {
                retain_local(
                    &mut local,
                    &mut metadata,
                    "empty",
                    "Fast Context 未命中，本次保留 Local 结果",
                );
            } else {
                let ranked = fuse_sections(
                    sections,
                    remote_sections,
                    &plan,
                    requested_max_results(request, 10),
                );
                metadata["enhancement_state"] = "completed".into();
                metadata["sources"] =
                    serde_json::json!(ranked.iter().map(|candidate| serde_json::json!({
                    "location": candidate.section.location, "backend": candidate.section.backend,
                })).collect::<Vec<_>>());
                local.text = format_workspace_sections(
                    &ranked,
                    "[sou hybrid] Local 与 Fast Context 已融合",
                    None,
                );
                local.hit_count = ranked.len();
                local.backend = BACKEND_HYBRID.to_string();
                local.fusion = Some("local_fast_context_rrf".to_string());
            }
        }
        Err(error) => {
            let state = if error.starts_with("budget:") {
                "timeout"
            } else {
                "error"
            };
            retain_local(
                &mut local,
                &mut metadata,
                state,
                &format!(
                    "Fast Context 增强结束，本次保留 Local 结果：{}",
                    diagnostic_summary(&error)
                ),
            );
        }
    }
    local.retrieval = Some(metadata);
    local.duration_ms = started.elapsed().as_millis() as u64;
    Ok(local)
}

async fn within_budget<T>(
    budget: Duration,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    // 中文说明：外层预算包含鉴权、全部轮次及重试，独立于 Fast Context 单次请求超时。
    tokio::time::timeout(budget, future)
        .await
        .map_err(|_| format!("budget: 增强总预算 {}ms 已用尽", budget.as_millis()))?
}

fn retain_local(
    local: &mut BackendRunResult,
    metadata: &mut serde_json::Value,
    state: &str,
    reason: &str,
) {
    metadata["enhancement_state"] = state.into();
    local.degraded = true;
    local.fallback_reason = Some(match local.fallback_reason.take() {
        Some(previous) => format!("{}；{}", previous, reason),
        None => reason.to_string(),
    });
}

fn enhancement_reason(
    plan: &QueryPlan,
    sections: &[SouSection],
    local_only: bool,
) -> Result<&'static str, &'static str> {
    if local_only {
        return Err("local_only_file");
    }
    if sections.is_empty() {
        return Ok("no_local_hits");
    }
    if plan.cross_module {
        return Ok("cross_module_query");
    }
    let combined = sections
        .iter()
        .filter(|section| {
            plan.intent != query::SearchIntent::Code
                || !query::is_document(&split_location_range(&section.location).0.to_lowercase())
        })
        .map(|section| format!("{}\n{}", section.location, section.excerpt))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    if !plan.files.is_empty() {
        let all_files = plan
            .files
            .iter()
            .all(|file| combined.contains(&file.to_lowercase()));
        if !all_files {
            return Ok("missing_requested_file");
        }
        if plan.identifiers.is_empty() {
            return Err("exact_file");
        }
    }
    if !plan.identifiers.is_empty()
        && plan.relevance("", &combined).identifier_hits == plan.identifiers.len()
    {
        return Err("exact_identifiers");
    }
    if plan.identifiers.is_empty()
        && !plan.query.trim().is_empty()
        && combined.contains(plan.query.trim())
    {
        return Err("exact_phrase");
    }
    Ok("insufficient_local_evidence")
}

fn candidate_context(root: &Path, sections: &[SouSection]) -> String {
    let mut text = String::new();
    let mut files = HashSet::new();
    for section in sections {
        let (path, range) = split_location_range(&section.location);
        let Ok(path) = Path::new(path).canonicalize() else {
            continue;
        };
        if !path.starts_with(root)
            || local::path_ignored(root, &path)
            || !files.insert(path.clone())
        {
            continue;
        }
        if files.len() > 3 {
            break;
        }
        let relative = path.strip_prefix(root).unwrap();
        let candidate = format!(
            "\n/codebase/{}:{}\n{}\n",
            normalize_path(relative),
            range.unwrap_or(""),
            section.excerpt
        );
        let remaining = 4000usize.saturating_sub(text.chars().count());
        text.extend(candidate.chars().take(remaining));
        if text.chars().count() >= 4000 {
            break;
        }
    }
    text
}

fn fuse_sections(
    local: Vec<SouSection>,
    fast: Vec<SouSection>,
    plan: &QueryPlan,
    limit: usize,
) -> Vec<RankedWorkspaceSection> {
    let mut ranked = [local, fast]
        .into_iter()
        .flat_map(|sections| {
            sections
                .into_iter()
                .enumerate()
                .map(|(rank, section)| RankedWorkspaceSection {
                    section,
                    retrieval_score: 1.0 / (61.0 + rank as f64),
                    exact_match: false,
                    coverage: 0,
                })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        exact_priority(plan, &right.section)
            .cmp(&exact_priority(plan, &left.section))
            .then_with(|| right.retrieval_score.total_cmp(&left.retrieval_score))
    });
    let mut merged: Vec<RankedWorkspaceSection> = Vec::new();
    for candidate in ranked {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| overlapping(&existing.section, &candidate.section))
        {
            let mut lines = numbered_lines(&candidate.section.excerpt);
            lines.extend(numbered_lines(&existing.section.excerpt));
            if let (Some(start), Some(end)) = (
                lines.keys().next().copied(),
                lines.keys().next_back().copied(),
            ) {
                let path = split_location_range(&existing.section.location)
                    .0
                    .to_string();
                existing.section.location = format!("{path}:{start}-{end}");
                existing.section.excerpt = lines
                    .into_iter()
                    .map(|(line, text)| format!("L{line}:{text}"))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            if !existing
                .section
                .backend
                .split('+')
                .any(|backend| backend == candidate.section.backend)
            {
                existing.retrieval_score += candidate.retrieval_score;
                existing.section.backend = "local+fast_context".to_string();
            }
        } else {
            merged.push(candidate);
        }
    }
    merged.sort_by(|left, right| {
        exact_priority(plan, &right.section)
            .cmp(&exact_priority(plan, &left.section))
            .then_with(|| right.retrieval_score.total_cmp(&left.retrieval_score))
    });
    // 中文说明：先覆盖不同文件，再用同文件的独立片段补足 TopK。
    let mut paths = HashSet::new();
    let mut extra = Vec::new();
    let mut result = Vec::new();
    for candidate in merged {
        if paths.insert(
            split_location_range(&candidate.section.location)
                .0
                .to_lowercase(),
        ) {
            result.push(candidate);
        } else {
            extra.push(candidate);
        }
    }
    result.extend(extra);
    result.truncate(limit.max(1));
    result
}

fn numbered_lines(excerpt: &str) -> BTreeMap<usize, String> {
    excerpt
        .lines()
        .filter_map(|line| {
            let (number, text) = line.strip_prefix('L')?.split_once(':')?;
            Some((number.parse().ok()?, text.to_string()))
        })
        .collect()
}

fn exact_priority(plan: &QueryPlan, section: &SouSection) -> (bool, bool, bool) {
    let path = split_location_range(&section.location).0;
    let evidence = plan.relevance(path, &section.excerpt);
    (
        evidence.file_match,
        evidence.definition_hits > 0,
        !plan.identifiers.is_empty() && evidence.identifier_hits == plan.identifiers.len(),
    )
}

fn overlapping(left: &SouSection, right: &SouSection) -> bool {
    let (left_path, _) = split_location_range(&left.location);
    let (right_path, _) = split_location_range(&right.location);
    let normalize = |path: &str| {
        path.replace('\\', "/")
            .trim_start_matches("//?/")
            .to_lowercase()
    };
    if normalize(left_path) != normalize(right_path) {
        return false;
    }
    let left = numbered_lines(&left.excerpt);
    let right = numbered_lines(&right.excerpt);
    match (
        left.keys().next(),
        left.keys().next_back(),
        right.keys().next(),
        right.keys().next_back(),
    ) {
        (Some(a), Some(b), Some(c), Some(d)) => a <= d && c <= b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "由 test-sou-hybrid.ps1 显式传入本次验收项目及固定查询"]
    async fn real_queries_from_env() {
        let suite: serde_json::Value =
            serde_json::from_str(&std::env::var("SANSHU_SOU_HYBRID_QUERIES").unwrap()).unwrap();
        let mut results = Vec::new();
        for case in suite.as_array().unwrap() {
            for attempt in 0..3 {
                let request = SouRequest {
                    project_root_path: case["root"].as_str().unwrap().to_string(),
                    query: case["query"].as_str().unwrap().to_string(),
                    backend: Some(case["backend"].as_str().unwrap().to_string()),
                    intent: Default::default(),
                    tree_depth: None,
                    max_turns: Some(2),
                    max_commands: Some(4),
                    max_results: Some(5),
                    timeout_ms: Some(8000),
                    exclude_paths: None,
                };
                // 中文说明：执行正式检索分支和 MCP 序列化，避开测试二进制引入无清单的桌面对话框依赖。
                let config = SouRuntimeConfig::load().unwrap();
                let output = if case["backend"] == "hybrid" {
                    run(&request, &config).await
                } else {
                    run_local(&request, &config).await
                }
                .unwrap();
                let result =
                    backend_success_result(output, case["backend"].as_str().unwrap(), true);
                assert_ne!(result.is_error, Some(true));
                let text = call_result_text(&result);
                let sections = parse_sou_sections(&text, "local");
                let expected = case["expected"].as_str().unwrap();
                let hit = sections.iter().any(|section| {
                    split_location_range(&section.location)
                        .0
                        .replace('\\', "/")
                        .ends_with(expected)
                });
                let metadata = result.structured_content.unwrap();
                results.push(serde_json::json!({"case": case["name"], "backend": case["backend"], "attempt": attempt,
                    "expected_hit": hit, "metadata": metadata,
                    "locations": sections.iter().map(|section| &section.location).collect::<Vec<_>>() }));
                assert!(hit, "固定查询目标未进入 Top5: {} / {}", case["name"], text);
                if case["name"] == "implementation" {
                    assert!(
                        split_location_range(&sections[0].location)
                            .0
                            .replace('\\', "/")
                            .ends_with(expected),
                        "查询样例不应挤掉真实实现的首位"
                    );
                }
            }
        }
        println!("SOU_HYBRID_REPORT={}", serde_json::json!(results));
    }
    fn section(path: &str, start: usize, end: usize, backend: &str) -> SouSection {
        SouSection {
            location: format!("{path}:{start}-{end}"),
            backend: backend.to_string(),
            excerpt: (start..=end)
                .map(|line| format!("L{line}:fn ensure_watcher() {{}}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    #[test]
    fn evidence_controls_enhancement_and_local_only_wins() {
        let plan = QueryPlan::new("ensure_watcher", query::SearchIntent::Auto);
        let sections = vec![section("src/local.rs", 1, 4, "local")];
        assert_eq!(
            enhancement_reason(&plan, &sections, false),
            Err("exact_identifiers")
        );
        assert_eq!(enhancement_reason(&plan, &[], false), Ok("no_local_hits"));
        let cross = QueryPlan::new("ensure_watcher 调用链", query::SearchIntent::Auto);
        assert_eq!(
            enhancement_reason(&cross, &sections, false),
            Ok("cross_module_query")
        );
        assert_eq!(
            enhancement_reason(&cross, &sections, true),
            Err("local_only_file")
        );
    }

    #[test]
    fn remote_seed_excludes_ignored_files_and_has_a_character_budget() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join(".gitignore"), "/docs/\n").unwrap();
        fs::write(temp.path().join("docs/private.md"), "local-only-doc").unwrap();
        fs::write(temp.path().join("source.rs"), "fn ensure_watcher() {}\n").unwrap();
        let root = temp.path().canonicalize().unwrap();
        let sections = vec![
            SouSection {
                backend: "local".to_string(),
                location: format!("{}:1-1", root.join("docs/private.md").display()),
                excerpt: "local-only-doc".to_string(),
            },
            SouSection {
                backend: "local".to_string(),
                location: format!("{}:1-1", root.join("source.rs").display()),
                excerpt: "源码".repeat(3000),
            },
        ];
        let seed = candidate_context(&root, &sections);
        assert!(!seed.contains("local-only-doc"));
        assert_eq!(seed.chars().count(), 4000);
    }

    #[test]
    fn overlapping_sources_merge_and_keep_one_global_limit() {
        let result = fuse_sections(
            vec![section("src/local.rs", 1, 4, "local")],
            vec![
                section("src/local.rs", 3, 6, "fast_context"),
                section("src/other.rs", 1, 2, "fast_context"),
            ],
            &QueryPlan::new("ensure_watcher", query::SearchIntent::Auto),
            2,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].section.location, "src/local.rs:1-6");
        assert_eq!(result[0].section.backend, "local+fast_context");
        let text = format_workspace_sections(&result, "[sou hybrid] 已融合", None);
        let parsed = parse_sou_sections(&text, "hybrid");
        assert!(parsed
            .iter()
            .all(|section| !section.excerpt.contains("[sou hybrid]")));
    }

    #[tokio::test]
    async fn outer_budget_stops_all_remote_work() {
        let result = within_budget(Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, String>(())
        })
        .await;
        assert!(result.unwrap_err().starts_with("budget:"));
    }
}
