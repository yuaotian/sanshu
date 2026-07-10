use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use serde_json::json;
use std::borrow::Cow;
use std::sync::Arc;

use super::store::{get_plan_store, PlanError};
use super::types::PlanRequest;
use crate::log_important;

pub struct PlanTool;

impl PlanTool {
    pub fn get_tool_definition() -> Tool {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "update", "get", "clear"],
                    "description": "计划动作：set 完整替换，update 更新单项状态，get 查询，clear 清空"
                },
                "workspace": {
                    "type": "string",
                    "minLength": 1,
                    "description": "工作区根目录绝对路径"
                },
                "items": {
                    "type": "array",
                    "minItems": 1,
                    "description": "set 时必填的有序计划项；同一计划最多一个 in_progress",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "id": { "type": "string", "minLength": 1, "description": "稳定且唯一的计划项 ID" },
                            "text": { "type": "string", "minLength": 1, "description": "计划项文本" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["id", "text", "status"]
                    }
                },
                "id": {
                    "type": "string",
                    "minLength": 1,
                    "description": "update 时必填的计划项 ID"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "update 时必填的目标状态"
                }
            },
            "required": ["action", "workspace"]
        });

        let serde_json::Value::Object(schema_map) = schema else {
            panic!("plan schema 创建失败");
        };
        Tool {
            name: Cow::Borrowed("plan"),
            description: Some(Cow::Borrowed(
                "维护当前工作区的开发执行计划。开始前用 set 提交完整计划，开始和完成步骤时用 update 单向更新状态，可用 get 查询或 clear 清空。",
            )),
            input_schema: Arc::new(schema_map),
            annotations: None,
            icons: None,
            meta: None,
            output_schema: None,
            title: Some("开发计划跟踪".to_string()),
        }
    }

    pub async fn execute(request: PlanRequest) -> Result<CallToolResult, McpError> {
        let action = request.action;
        let result = get_plan_store()
            .and_then(|store| store.execute(request))
            .map_err(Self::map_error)?;

        log_important!(
            info,
            "[plan] 动作完成: action={}, changed={}, completed={}, total={}, workspace={}",
            action.as_str(),
            result.changed,
            result.summary.completed,
            result.summary.total,
            result.workspace
        );

        let structured_content = serde_json::to_value(&result).map_err(|error| {
            McpError::internal_error(format!("序列化计划结果失败：{}", error), None)
        })?;
        let summary = format!(
            "计划操作完成：action={}，changed={}，进度={}/{}",
            result.action, result.changed, result.summary.completed, result.summary.total
        );
        Ok(CallToolResult {
            content: vec![Content::text(summary)],
            is_error: Some(false),
            structured_content: Some(structured_content),
            meta: None,
        })
    }

    fn map_error(error: PlanError) -> McpError {
        match error {
            PlanError::Validation(message) | PlanError::Conflict(message) => {
                McpError::invalid_params(message, None)
            }
            PlanError::Corrupt(message) | PlanError::Storage(message) => {
                McpError::internal_error(message, None)
            }
        }
    }
}
