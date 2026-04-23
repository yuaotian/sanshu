// UI/UX MCP 工具请求类型

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UiuxOutputFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UiuxLang {
    Zh,
    En,
}

impl UiuxLang {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiuxLang::Zh => "zh",
            UiuxLang::En => "en",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiuxAction {
    Beautify,
    Describe,
    Audit,
    DesignSystem,
}

impl UiuxAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiuxAction::Beautify => "beautify",
            UiuxAction::Describe => "describe",
            UiuxAction::Audit => "audit",
            UiuxAction::DesignSystem => "design_system",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiuxRequest {
    pub query: String,
    #[serde(default)]
    pub action: Option<UiuxAction>,
    #[serde(default)]
    pub project_root_path: Option<String>,
    #[serde(default)]
    pub current_file_path: Option<String>,
    #[serde(default)]
    pub context_query: Option<String>,
    #[serde(default)]
    pub append_project_context: Option<bool>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub output_format: Option<UiuxOutputFormat>,
    #[serde(default)]
    pub lang: Option<UiuxLang>,
}
