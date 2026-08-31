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
async fn uiux_beautify_supports_explicit_local_ab_baseline() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "glassmorphism 毛玻璃 金融仪表盘",
            "action": "beautify",
            "knowledge_backend": "local",
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
        Some("local_bm25")
    );
    assert_eq!(
        v["data"]["retrieval"]["knowledge_diagnostics"]["version"].as_str(),
        Some("v2.15.0")
    );
    assert_eq!(
        v["data"]["retrieval"]["knowledge_diagnostics"]["engine"].as_str(),
        Some("structured_bm25_v1")
    );
    assert_eq!(v["data"]["retrieval"]["degraded"].as_bool(), Some(false));
    assert_eq!(
        v["data"]["retrieval"]["requested_knowledge_backend"].as_str(),
        Some("local")
    );
    assert_eq!(
        v["data"]["retrieval"]["knowledge_hit_count"].as_u64(),
        v["data"]["uiux_hits"]
            .as_array()
            .map(|hits| hits.len() as u64)
    );
    assert!(v["data"]["retrieval"]["knowledge_duration_ms"].is_u64());
    assert_eq!(
        v["data"]["retrieval"]["project_context_appended"].as_bool(),
        Some(false)
    );
    assert!(v["data"]["prompt"]
        .as_str()
        .unwrap_or_default()
        .contains("页面美化提示词"));
    assert!(v["data"]["uiux_hits"]
        .as_array()
        .map(|arr| !arr.is_empty())
        .unwrap_or(false));
}

#[tokio::test]
async fn uiux_auto_uses_local_bm25_for_long_chinese_narrative() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "查找鼠标动效主题美化：黑洞吞噬小星球、地球月球卫星战舰公转系统、深邃暗黑多层星空、巨手破空抓握地球拖入裂缝，保持 KISS/YAGNI 极简几何科技感与高性能 Direct2D/Canvas 实现",
            "action": "beautify",
            "knowledge_backend": "auto",
            "append_project_context": false,
            "max_results": 3,
            "output_format": "json"
        }),
    )
    .await
    .expect("uiux auto 调用应成功");
    let value = parse_uiux_json(&extract_first_text(&result));
    let retrieval = &value["data"]["retrieval"];

    assert_eq!(retrieval["knowledge_source"].as_str(), Some("local_bm25"));
    assert_eq!(retrieval["degraded"].as_bool(), Some(false));
    assert_eq!(
        retrieval["knowledge_diagnostics"]["status"].as_str(),
        Some("matched")
    );
    assert_eq!(retrieval["knowledge_hit_count"].as_u64(), Some(3));
    let domains = retrieval["knowledge_diagnostics"]["domains"]
        .as_array()
        .expect("应返回知识域诊断");
    for expected in ["style", "product", "motion"] {
        assert!(
            domains
                .iter()
                .any(|domain| domain.as_str() == Some(expected)),
            "应包含 {expected} 知识域，实际为: {domains:?}"
        );
    }
    assert!(value["data"]["uiux_hits"]
        .as_array()
        .is_some_and(|hits| hits.iter().all(|hit| hit["location"]
            .as_str()
            .is_some_and(|location| location.contains("ui-ux-pro-max-v2.15.0/data/")))));
    let hits_text = value["data"]["uiux_hits"].to_string();
    assert!(hits_text.contains("HUD / Sci-Fi FUI"));
    assert!(hits_text.contains("Space Tech / Aerospace"));
}

#[tokio::test]
async fn uiux_describe_returns_single_tool_contract() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "beauty spa wellness service elegant",
            "action": "describe",
            "knowledge_backend": "local",
            "output_format": "json"
        }),
    )
    .await
    .expect("uiux describe 调用应成功");
    let text = extract_first_text(&result);
    let v = parse_uiux_json(&text);

    assert_eq!(v["meta"]["tool"].as_str(), Some("uiux"));
    assert_eq!(v["data"]["action"].as_str(), Some("describe"));
    assert!(v["data"]["prompt"]
        .as_str()
        .unwrap_or_default()
        .contains("UI 描述提示词"));
}

#[tokio::test]
async fn uiux_design_system_keeps_project_context_disabled_without_project_root() {
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "后台管理面板 设计系统",
            "action": "design_system",
            "knowledge_backend": "local",
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

#[test]
fn uiux_schema_exposes_request_level_knowledge_backend() {
    let definitions = UiuxTool::get_tool_definitions();
    let schema = definitions
        .first()
        .expect("uiux 工具定义应存在")
        .input_schema
        .as_ref();
    let property = schema
        .get("properties")
        .and_then(|properties| properties.get("knowledge_backend"))
        .expect("schema 应暴露 knowledge_backend");

    assert_eq!(
        property.get("enum").and_then(|value| value.as_array()),
        Some(&vec![json!("auto"), json!("fast_context"), json!("local")])
    );
}
