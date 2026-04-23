use rmcp::model::CallToolResult;
use serde_json::json;

use sanshu::tools::UiuxTool;

fn extract_first_text(result: &CallToolResult) -> String {
    let v = serde_json::to_value(&result.content).expect("Content 应可序列化为 JSON");
    let arr = v.as_array().expect("CallToolResult.content 应为数组");
    let first = arr.first().expect("CallToolResult.content 不应为空");

    if let Some(text) = first.get("text").and_then(|x| x.as_str()) {
        return text.to_string();
    }

    if let Some(text) = first.get("data").and_then(|x| x.as_str()) {
        return text.to_string();
    }

    panic!("无法从 content 提取文本: {}", v);
}

fn parse_uiux_json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("uiux 工具应输出 JSON 文本")
}

#[tokio::test]
async fn uiux_beautify_uses_local_markdown_fallback_without_project_root() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "glassmorphism 毛玻璃 金融仪表盘",
            "action": "beautify",
            "output_format": "json"
        }),
    )
    .await
    .expect("uiux 调用应成功");
    let text = extract_first_text(&result);
    let v = parse_uiux_json(&text);

    assert_eq!(v["meta"]["tool"].as_str(), Some("uiux"));
    assert_eq!(v["data"]["action"].as_str(), Some("beautify"));
    assert_eq!(
        v["data"]["retrieval"]["knowledge_source"].as_str(),
        Some("local_markdown")
    );
    assert_eq!(v["data"]["retrieval"]["degraded"].as_bool(), Some(true));
    assert!(v["data"]["prompt"].as_str().unwrap_or_default().contains("页面美化提示词"));
    assert!(v["data"]["uiux_hits"].as_array().map(|arr| !arr.is_empty()).unwrap_or(false));
}

#[tokio::test]
async fn uiux_describe_returns_single_tool_contract() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "beauty spa wellness service elegant",
            "action": "describe",
            "output_format": "json"
        }),
    )
    .await
    .expect("uiux describe 调用应成功");
    let text = extract_first_text(&result);
    let v = parse_uiux_json(&text);

    assert_eq!(v["meta"]["tool"].as_str(), Some("uiux"));
    assert_eq!(v["data"]["action"].as_str(), Some("describe"));
    assert!(v["data"]["prompt"].as_str().unwrap_or_default().contains("目标 UI 风格"));
}

#[tokio::test]
async fn uiux_design_system_keeps_project_context_disabled_without_project_root() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "后台管理面板 设计系统",
            "action": "design_system",
            "output_format": "json"
        }),
    )
    .await
    .expect("uiux design_system 调用应成功");
    let text = extract_first_text(&result);
    let v = parse_uiux_json(&text);

    assert_eq!(v["data"]["action"].as_str(), Some("design_system"));
    assert_eq!(
        v["data"]["retrieval"]["project_context_enabled"].as_bool(),
        Some(false)
    );
    assert!(v["text"].as_str().unwrap_or_default().contains("提示词"));
}
