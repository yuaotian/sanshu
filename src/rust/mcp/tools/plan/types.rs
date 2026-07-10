use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PLAN_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    Set,
    Update,
    Get,
    Clear,
}

impl PlanAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Update => "update",
            Self::Get => "get",
            Self::Clear => "clear",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

impl PlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanItem {
    pub id: String,
    pub text: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    pub action: PlanAction,
    pub workspace: String,
    #[serde(default)]
    pub items: Option<Vec<PlanItem>>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<PlanStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanFile {
    pub version: u32,
    pub workspace: String,
    pub items: Vec<PlanItem>,
}

impl PlanFile {
    pub fn empty(workspace: String) -> Self {
        Self {
            version: PLAN_FILE_VERSION,
            workspace,
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanSummary {
    pub completed: usize,
    pub total: usize,
    pub all_completed: bool,
}

impl PlanSummary {
    pub fn from_items(items: &[PlanItem]) -> Self {
        let completed = items
            .iter()
            .filter(|item| item.status == PlanStatus::Completed)
            .count();
        let total = items.len();
        Self {
            completed,
            total,
            all_completed: total > 0 && completed == total,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlanResult {
    pub action: String,
    pub workspace: String,
    pub changed: bool,
    pub items: Vec<PlanItem>,
    pub summary: PlanSummary,
}

impl PlanResult {
    pub fn new(action: PlanAction, workspace: String, changed: bool, items: Vec<PlanItem>) -> Self {
        let summary = PlanSummary::from_items(&items);
        Self {
            action: action.as_str().to_string(),
            workspace,
            changed,
            items,
            summary,
        }
    }
}
