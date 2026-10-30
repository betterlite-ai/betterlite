use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::iter::once;
use std::str::{Utf8Error, from_utf8};

const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub model_max_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub temp_budget_increase: Option<Decimal>,
    #[serde(default)]
    pub temp_budget_expiry: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMBudgetTableFull {
    #[serde(flatten)]
    pub budget: LiteLLMBudgetTable,
    #[serde(default)]
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl LiteLLMBudgetTable {
    #[must_use]
    pub fn active_temp_budget_increase(&self, now: DateTime<Utc>) -> Decimal {
        match (self.temp_budget_increase, self.temp_budget_expiry) {
            (Some(amount), Some(expiry)) if expiry > now => amount,
            _ => Decimal::ZERO,
        }
    }
    #[must_use]
    pub fn effective_max_budget(&self, now: DateTime<Utc>) -> Option<Decimal> {
        let budget = self.max_budget?;
        budget.checked_add(self.active_temp_budget_increase(now))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    once(values).next()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonData {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub user: Option<JsonData>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    pub user_email: Option<JsonData>,
}
impl OrganizationMembership {
    pub fn populate_user_email(&mut self) {
        if self.user_email.is_none()
            && let Some(JsonData::Object(user)) = self.user.as_ref()
        {
            self.user_email = user.get("user_email").cloned();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetPermissionDecodeError {
    #[error("stored row is {len} bytes, over the {MAX_ROW_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("stored row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored row exceeds JSON depth {MAX_JSON_DEPTH}")]
    TooDeep,
    #[error("stored row is invalid JSON")]
    Json(#[source] serde_json::Error),
    #[error("stored row kind is invalid")]
    UnknownKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    let text = from_utf8(bytes).map_err(BudgetPermissionDecodeError::InvalidUtf8)?;
    if json_depth(bytes) > MAX_JSON_DEPTH {
        return Err(BudgetPermissionDecodeError::TooDeep);
    }
    serde_json::from_str(text).map_err(BudgetPermissionDecodeError::Json)
}
fn json_depth(bytes: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut maximum = 0_usize;
    for byte in bytes {
        match byte {
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                maximum = maximum.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    maximum
}

#[path = "budget_permission_rows/tests.rs"]
mod tests;
