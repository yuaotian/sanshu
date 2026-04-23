// UI/UX Pro Max 统一响应结构
// 用于所有 uiux_* 工具输出的 JSON 结构化响应

use serde::Serialize;

use super::types::UiuxLang;

#[derive(Debug, Clone, Serialize)]
pub struct UiuxResponse<T> {
    pub meta: UiuxMeta,
    pub data: T,
    pub text: String,
    pub errors: Vec<UiuxError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiuxMeta {
    pub tool: String,
    pub lang: String,
    pub request_id: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiuxError {
    pub code: String,
    pub message: String,
}

impl UiuxError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

impl<T> UiuxResponse<T> {
    pub fn new(tool: &str, lang: UiuxLang, data: T, text: String, errors: Vec<UiuxError>) -> Self {
        Self {
            meta: UiuxMeta {
                tool: tool.to_string(),
                lang: lang.as_str().to_string(),
                request_id: None,
                version: "v2".to_string(),
            },
            data,
            text,
            errors,
        }
    }
}
