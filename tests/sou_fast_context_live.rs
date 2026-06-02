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

#[tokio::test]
#[ignore = "需要本机 Windsurf 登录或 WINDSURF_API_KEY，并会访问 Windsurf 远端 API"]
async fn live_fast_context_search_smoke() {
    if std::env::var("SANSHU_LIVE_FAST_CONTEXT").ok().as_deref() != Some("1") {
        eprintln!("跳过 live fast-context 验证：请设置 SANSHU_LIVE_FAST_CONTEXT=1 后重跑。");
        return;
    }

    let project_root = std::env::var("SANSHU_LIVE_FAST_CONTEXT_PROJECT")
        .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_string())
        .replace('\\', "/");
    let query = std::env::var("SANSHU_LIVE_FAST_CONTEXT_QUERY").unwrap_or_else(|_| {
        "定位 sou fast_context Rust SearchOptions 和输出格式化实现".to_string()
    });
    let tree_depth = if std::env::var("SANSHU_LIVE_FAST_CONTEXT_PROJECT").is_ok() {
        3
    } else {
        1
    };
    let result = SouTool::search_context(SouRequest {
        project_root_path: project_root,
        query,
        backend: Some("fast_context".to_string()),
        tree_depth: Some(tree_depth),
        max_turns: Some(1),
        max_results: Some(3),
        max_commands: Some(3),
        timeout_ms: Some(60_000),
        exclude_paths: Some(vec![
            "target".to_string(),
            "node_modules".to_string(),
            "dist".to_string(),
            ".git".to_string(),
        ]),
    })
    .await
    .expect("sou fast_context 调用不应出现 MCP 内部错误");

    let text = first_text(&result);
    assert!(
        !result.is_error.unwrap_or(false),
        "fast-context 返回错误：{}",
        text
    );
    assert!(
        text.contains("Path:")
            || text.contains("No relevant files found.")
            || text.contains("grep keywords:"),
        "fast-context 返回了无法识别的输出：{}",
        text
    );
    assert!(
        text.contains("[fast-context stats]"),
        "fast-context 输出缺少命中率统计：{}",
        text
    );
    if let Some(stats_line) = text
        .lines()
        .find(|line| line.contains("[fast-context stats]"))
    {
        println!("live stats: {stats_line}");
    }
}
