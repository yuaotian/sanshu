// fast-context 优化后的端到端 benchmark
//
// 默认 ignore；运行方法：
//   $env:SANSHU_LIVE_FAST_CONTEXT="1"
//   $env:SANSHU_BENCH_TARGET="E:/ProjectCode/C++Code/omni-mouse-plus"
//   cargo test --test sou_fast_context_bench -- --ignored --nocapture
//
// 输出每个查询的耗时 (ms)、返回文件数、命中文件列表，供人工核验准确性。

use std::time::Instant;

use rmcp::model::CallToolResult;
use sanshu::tools::sou::{SouRequest, SouTool};

fn first_text(result: &CallToolResult) -> String {
    let value = serde_json::to_value(&result.content).expect("CallToolResult content 应可序列化");
    value
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| {
            item.get("text")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("data").and_then(|v| v.as_str()))
        })
        .unwrap_or_default()
        .to_string()
}

/// 解析输出中所有 `Path: xxx` 行
fn extract_paths(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("Path: ")
                .map(|s| s.trim().to_string())
        })
        .collect()
}

struct BenchCase {
    name: &'static str,
    query: &'static str,
    /// 应当出现在命中文件路径中的关键词（任一命中即视为相关）
    expected_keywords: &'static [&'static str],
}

#[tokio::test]
#[ignore = "需要 SANSHU_LIVE_FAST_CONTEXT=1 + SANSHU_BENCH_TARGET 指定目标项目"]
async fn bench_fast_context_against_real_project() {
    if std::env::var("SANSHU_LIVE_FAST_CONTEXT").ok().as_deref() != Some("1") {
        eprintln!("跳过 benchmark：未设置 SANSHU_LIVE_FAST_CONTEXT=1");
        return;
    }
    let target = match std::env::var("SANSHU_BENCH_TARGET") {
        Ok(v) if !v.trim().is_empty() => v.replace('\\', "/"),
        _ => {
            eprintln!("跳过 benchmark：未设置 SANSHU_BENCH_TARGET");
            return;
        }
    };

    let cases = vec![
        BenchCase {
            name: "具体类查询",
            query: "GestureRecognizer 类如何识别用户的手势轨迹？核心算法和模板匹配实现位置",
            expected_keywords: &["GestureRecognizer", "gesture"],
        },
        BenchCase {
            name: "跨模块链路",
            query: "鼠标手势识别后如何触发并执行对应的动作？gesture 模块和 action 模块之间的调用链",
            expected_keywords: &["ActionExecutor", "action", "gesture"],
        },
        BenchCase {
            name: "模糊语义",
            query: "Locate code that encodes captured screenshots and writes them into the system clipboard (ImageCodec, ClipboardService).",
            expected_keywords: &["Capture", "Clipboard", "ImageCodec", "capture"],
        },
    ];

    println!("\n=== fast-context benchmark on {} ===\n", target);
    println!(
        "{:<14} {:<10} {:<6} {:<8} {:<32}",
        "Case", "elapsed_ms", "files", "hit?", "first_paths"
    );
    println!("{}", "-".repeat(80));

    let mut total_ms = 0u128;
    let mut total_files = 0usize;
    let mut total_hits = 0usize;

    for case in &cases {
        let started = Instant::now();
        let result = SouTool::search_context(SouRequest {
            project_root_path: target.clone(),
            query: case.query.to_string(),
            backend: Some("fast_context".to_string()),
            tree_depth: Some(3),
            max_turns: Some(3),
            max_results: Some(8),
            max_commands: Some(4),
            timeout_ms: Some(60_000),
            exclude_paths: Some(vec![
                "build".to_string(),
                "target".to_string(),
                ".git".to_string(),
                "logs".to_string(),
                "node_modules".to_string(),
            ]),
        })
        .await
        .expect("sou fast_context 调用不应出现 MCP 内部错误");

        let elapsed_ms = started.elapsed().as_millis();
        let text = first_text(&result);
        let is_err = result.is_error.unwrap_or(false);
        let paths = extract_paths(&text);
        let hit = paths.iter().any(|p| {
            case.expected_keywords
                .iter()
                .any(|kw| p.to_lowercase().contains(&kw.to_lowercase()))
        });
        let first_preview = paths
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");

        println!(
            "{:<14} {:<10} {:<6} {:<8} {:<32}",
            case.name,
            elapsed_ms,
            paths.len(),
            if hit { "YES" } else { "NO" },
            first_preview
        );

        if is_err {
            println!("  ⚠ 返回错误: {}", text.chars().take(300).collect::<String>());
        } else {
            println!("  全部命中文件: {:?}", paths);
        }

        total_ms += elapsed_ms;
        total_files += paths.len();
        if hit {
            total_hits += 1;
        }
    }

    println!("{}", "-".repeat(80));
    println!(
        "合计耗时: {} ms | 平均: {} ms | 命中文件总数: {} | 准确性命中: {}/{}",
        total_ms,
        total_ms / cases.len() as u128,
        total_files,
        total_hits,
        cases.len()
    );
}
