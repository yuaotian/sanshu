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
async fn uiux_local_ocr_audit_exposes_relevant_knowledge_and_retrieval_diagnostics() {
    // 中文说明：固定原始故障请求并关闭项目上下文，验证知识检索自身的完整 JSON 契约。
    let query = "OCR 纯屏幕文字识别二级菜单工具栏面板美化与组件体验重塑";
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": query,
            "action": "audit",
            "knowledge_backend": "local",
            "append_project_context": false,
            "max_results": 3,
            "lang": "zh"
        }),
    )
    .await
    .expect("OCR 审查请求应完成");
    let value = parse_uiux_json(&extract_first_text(&result));
    let retrieval = &value["data"]["retrieval"];
    let diagnostics = &retrieval["knowledge_diagnostics"];
    let hits = value["data"]["uiux_hits"]
        .as_array()
        .expect("应返回知识片段数组");

    assert!(!hits.is_empty() && hits.len() <= 3);
    assert_eq!(
        retrieval["knowledge_hit_count"].as_u64(),
        Some(hits.len() as u64)
    );
    assert_eq!(retrieval["requested_knowledge_backend"], "local");
    assert_eq!(retrieval["knowledge_backend_source"], "request");
    assert_eq!(retrieval["knowledge_source"], "local_bm25");
    assert_eq!(retrieval["project_context_enabled"], false);
    assert_eq!(retrieval["project_context_appended"], false);
    assert_eq!(retrieval["queries"]["local_knowledge_query"], query);
    assert_eq!(diagnostics["status"], "matched");
    assert_eq!(diagnostics["reason"], "matched");
    assert!(diagnostics["candidate_count"].as_u64().unwrap_or_default() >= hits.len() as u64);
    assert!(hits.iter().any(|hit| {
        hit["location"]
            .as_str()
            .is_some_and(|location| location.contains("/data/products.csv:"))
            && hit["excerpt"]
                .as_str()
                .is_some_and(|excerpt| excerpt.contains("Scanner & Document Manager"))
    }));
    assert!(hits.iter().any(|hit| {
        hit["location"]
            .as_str()
            .is_some_and(|location| location.contains("/data/ux-guidelines.csv:"))
    }));
    for field in ["domains", "searched_domains"] {
        let domains = diagnostics[field].as_array().expect("应保留领域诊断");
        assert!(domains.contains(&json!("product")));
        assert!(domains.contains(&json!("ux")));
    }
    let query_tokens = diagnostics["query_tokens"]
        .as_array()
        .expect("应返回主题词项");
    for token in ["ocr", "menu", "toolbar"] {
        assert!(query_tokens.contains(&json!(token)));
    }
    assert!(!query_tokens.contains(&json!("accessibility")));
    assert!(diagnostics["action_tokens"]
        .as_array()
        .expect("应返回动作词项")
        .contains(&json!("accessibility")));
    assert!(value["errors"]
        .as_array()
        .expect("应返回错误数组")
        .is_empty());
}

#[tokio::test]
async fn uiux_unrelated_audit_preserves_empty_knowledge_error_and_prompt_notice() {
    // 中文说明：审查动作补词仅参与排序，零主题证据时保持空结果与明确的知识缺失提示。
    let result = UiuxTool::call_tool(
        "uiux",
        json!({
            "query": "ZXQJ9471QX NOTKNOWLEDGE113",
            "action": "audit",
            "knowledge_backend": "local",
            "append_project_context": false,
            "max_results": 3
        }),
    )
    .await
    .expect("无关审查请求应返回结构化结果");
    let value = parse_uiux_json(&extract_first_text(&result));
    let retrieval = &value["data"]["retrieval"];
    let diagnostics = &retrieval["knowledge_diagnostics"];

    assert!(value["data"]["uiux_hits"]
        .as_array()
        .expect("应返回知识片段数组")
        .is_empty());
    assert_eq!(retrieval["knowledge_hit_count"], 0);
    assert_eq!(diagnostics["candidate_count"], 0);
    assert_eq!(diagnostics["reason"], "no_domain_terms");
    assert_eq!(diagnostics["status"], "matched_low_confidence");
    assert!(!diagnostics["action_tokens"]
        .as_array()
        .expect("审查动作仍应保留词项")
        .is_empty());
    assert!(value["errors"]
        .as_array()
        .expect("应返回错误数组")
        .iter()
        .any(|error| error["code"] == "uiux_knowledge_empty"));
    let prompt = value["data"]["prompt"]
        .as_str()
        .expect("应保留可解释的提示词");
    assert!(prompt.contains("本次未取得 UI/UX 知识片段"));
    assert!(!prompt.contains("# UI/UX 参考知识"));
}

#[tokio::test]
async fn uiux_auto_uses_local_engine_for_long_chinese_narrative() {
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

    assert!(matches!(
        retrieval["knowledge_source"].as_str(),
        Some("local_bm25" | "local_hybrid")
    ));
    assert_eq!(retrieval["degraded"].as_bool(), Some(false));
    assert!(retrieval["knowledge_diagnostics"]["status"]
        .as_str()
        .is_some_and(|status| status.starts_with("matched")));
    // 中文说明：资源压力导致语义让路时仍保留 BM25，属于既有 auto 回落契约。
    assert!(matches!(
        retrieval["knowledge_diagnostics"]["semantic_state"].as_str(),
        Some("ready" | "missing" | "loading" | "disabled" | "error" | "resource")
    ));
    assert_eq!(retrieval["knowledge_hit_count"].as_u64(), Some(3));
    let domains = retrieval["knowledge_diagnostics"]["domains"]
        .as_array()
        .expect("应返回知识域诊断");
    assert!(!domains.is_empty());
    assert!(value["data"]["uiux_hits"]
        .as_array()
        .is_some_and(|hits| hits.iter().all(|hit| hit["location"]
            .as_str()
            .is_some_and(|location| location.contains("ui-ux-pro-max-v2.15.0/data/")))));
    if retrieval["knowledge_source"].as_str() == Some("local_bm25") {
        let hits_text = value["data"]["uiux_hits"].to_string();
        assert!(hits_text.contains("HUD / Sci-Fi FUI"));
        assert!(hits_text.contains("Space Tech / Aerospace"));
    }
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
