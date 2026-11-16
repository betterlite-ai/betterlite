use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::{JsonData, Timestamp};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallPolicy {
    Trusted,
    #[default]
    Untrusted,
    DualLlm,
    Blocked,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInputPolicy {
    Trusted,
    #[default]
    Untrusted,
    Blocked,
}

impl ToolInputPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Blocked => "blocked",
        }
    }
}

// LiteLLM's ToolPolicyOverrideRow declares input_policy = "blocked", not the tool row's untrusted default
const fn blocked_input_policy() -> ToolInputPolicy {
    ToolInputPolicy::Blocked
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutputPolicy {
    Trusted,
    #[default]
    Untrusted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDiscoveryUser {
    pub user_id: String,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default)]
    pub user_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMToolTableRow {
    #[serde(default)]
    pub tool_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub input_policy: ToolInputPolicy,
    #[serde(default)]
    pub output_policy: ToolOutputPolicy,
    #[serde(default)]
    pub call_count: i64,
    #[serde(default)]
    pub assignments: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub key_hash: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub user: Option<ToolDiscoveryUser>,
    #[serde(default)]
    pub last_used_at: Option<Timestamp>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolListResponse {
    pub tools: Vec<LiteLLMToolTableRow>,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyUpdateRequest {
    pub tool_name: String,
    #[serde(default)]
    pub input_policy: Option<ToolInputPolicy>,
    #[serde(default)]
    pub output_policy: Option<ToolOutputPolicy>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_hash: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyUpdateResponse {
    pub tool_name: String,
    #[serde(default)]
    pub input_policy: Option<ToolInputPolicy>,
    #[serde(default)]
    pub output_policy: Option<ToolOutputPolicy>,
    pub updated: bool,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyOverrideRow {
    pub override_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_hash: Option<String>,
    #[serde(default = "blocked_input_policy")]
    pub input_policy: ToolInputPolicy,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyOption {
    pub value: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicyOptionsResponse {
    pub input_policies: Vec<ToolPolicyOption>,
    pub output_policies: Vec<ToolPolicyOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDetailResponse {
    pub tool: LiteLLMToolTableRow,
    #[serde(default)]
    pub overrides: Vec<ToolPolicyOverrideRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUsageLogEntry {
    pub id: String,
    pub timestamp: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub total_tokens: Option<i64>,
    #[serde(default)]
    pub input_snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUsageLogsResponse {
    pub logs: Vec<ToolUsageLogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpendEntry {
    pub tool_name: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(default)]
    pub call_count: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpendDailyEntry {
    pub date: String,
    pub tool_name: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(default)]
    pub call_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpendResponse {
    #[serde(default)]
    pub by_tool: Vec<ToolSpendEntry>,
    #[serde(default)]
    pub daily: Vec<ToolSpendDailyEntry>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}
