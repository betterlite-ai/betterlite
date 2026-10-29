use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_ROW_BYTES: usize = 1_048_576;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiteLLMBudgetTable {
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub soft_budget: Option<Decimal>,
    #[serde(default)]
    pub max_budget: Option<Decimal>,
    #[serde(default)]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub tpd_limit: Option<i64>,
    #[serde(default)]
    pub model_max_budget: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub temp_budget_increase: Option<Decimal>,
    #[serde(default)]
    pub temp_budget_expiry: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiteLLMBudgetTableFull {
    #[serde(flatten)]
    pub budget: LiteLLMBudgetTable,
    #[serde(default)]
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl LiteLLMBudgetTable {
    pub fn active_temp_budget_increase(&self, now: DateTime<Utc>) -> Decimal {
        match (self.temp_budget_increase, self.temp_budget_expiry) {
            (Some(amount), Some(expiry)) if expiry > now => amount,
            _ => Decimal::ZERO,
        }
    }
    pub fn effective_max_budget(&self, now: DateTime<Utc>) -> Option<Decimal> {
        let budget = self.max_budget?;
        Some(budget + self.active_temp_budget_increase(now))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiteLLMObjectPermissionTable {
    pub object_permission_id: String,
    #[serde(default = "empty_list")]
    pub mcp_servers: Option<Vec<String>>,
    #[serde(default = "empty_list")]
    pub mcp_access_groups: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_tool_permissions: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default = "empty_list")]
    pub vector_stores: Option<Vec<String>>,
    #[serde(default = "empty_list")]
    pub agents: Option<Vec<String>>,
    #[serde(default = "empty_list")]
    pub agent_access_groups: Option<Vec<String>>,
    #[serde(default = "empty_list")]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_toolsets: Option<Vec<String>>,
    #[serde(default = "empty_list")]
    pub blocked_tools: Option<Vec<String>>,
    #[serde(default = "empty_list")]
    pub search_tools: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_tool_search_enabled: Option<bool>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
}
fn empty_list() -> Option<Vec<String>> {
    let values = Vec::new();
    Some(values)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrganizationMembership {
    pub user_id: String,
    pub organization_id: String,
    #[serde(default)]
    pub user_role: Option<String>,
    #[serde(default)]
    pub spend: Decimal,
    #[serde(default)]
    pub budget_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub user: Option<Value>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    pub user_email: Option<Value>,
}
impl OrganizationMembership {
    pub fn populate_user_email(&mut self) {
        if self.user_email.is_none() {
            if let Some(Value::Object(user)) = self.user.as_ref() {
                self.user_email = user.get("user_email").cloned();
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetPermissionDecodeError {
    #[error("stored row is {len} bytes, over the {MAX_ROW_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("stored row is not valid UTF-8")]
    InvalidUtf8(#[source] std::str::Utf8Error),
    #[error("stored row exceeds JSON depth {MAX_JSON_DEPTH}")]
    TooDeep,
    #[error("stored row is invalid JSON")]
    Json(#[source] serde_json::Error),
    #[error("stored row kind is invalid")]
    UnknownKind,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "row")]
pub enum BudgetPermissionRow {
    Budget(LiteLLMBudgetTable),
    Full(LiteLLMBudgetTableFull),
    Permission(LiteLLMObjectPermissionTable),
    Membership(OrganizationMembership),
}
pub fn decode_budget_permission_rows(
    bytes: &[u8],
) -> Result<BudgetPermissionRow, BudgetPermissionDecodeError> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err(BudgetPermissionDecodeError::TooLarge { len: bytes.len() });
    }
    let text = std::str::from_utf8(bytes).map_err(BudgetPermissionDecodeError::InvalidUtf8)?;
    let value: Value = serde_json::from_str(text).map_err(BudgetPermissionDecodeError::Json)?;
    if json_depth(&value) > MAX_JSON_DEPTH {
        return Err(BudgetPermissionDecodeError::TooDeep);
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(BudgetPermissionDecodeError::UnknownKind)?;
    let row = value
        .get("row")
        .ok_or(BudgetPermissionDecodeError::UnknownKind)?
        .clone();
    match kind {
        "budget" => serde_json::from_value(row)
            .map(BudgetPermissionRow::Budget)
            .map_err(BudgetPermissionDecodeError::Json),
        "full" => serde_json::from_value(row)
            .map(BudgetPermissionRow::Full)
            .map_err(BudgetPermissionDecodeError::Json),
        "permission" => serde_json::from_value(row)
            .map(BudgetPermissionRow::Permission)
            .map_err(BudgetPermissionDecodeError::Json),
        "membership" => serde_json::from_value(row)
            .map(BudgetPermissionRow::Membership)
            .map_err(BudgetPermissionDecodeError::Json),
        _ => Err(BudgetPermissionDecodeError::UnknownKind),
    }
}
fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(json_depth).max().unwrap_or(0) + 1,
        Value::Object(values) => values.values().map(json_depth).max().unwrap_or(0) + 1,
        _ => 0,
    }
}

#[path = "budget_permission_rows/tests.rs"]
mod tests;
