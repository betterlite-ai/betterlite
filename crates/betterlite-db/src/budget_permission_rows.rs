use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};

const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Timestamp {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}
impl Timestamp {
    #[must_use]
    pub fn utc(self) -> DateTime<Utc> {
        match self {
            Self::Aware(value) => value.with_timezone(&Utc),
            Self::Naive(value) => value.and_utc(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMBudgetTable {
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub soft_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
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
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub temp_budget_increase: Option<Decimal>,
    #[serde(default)]
    pub temp_budget_expiry: Option<Timestamp>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BudgetError {
    #[error("budget arithmetic overflow")]
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMBudgetTableFull {
    #[serde(flatten)]
    pub budget: LiteLLMBudgetTable,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl LiteLLMBudgetTable {
    #[must_use]
    pub fn active_temp_budget_increase(&self, now: DateTime<Utc>) -> Decimal {
        match (self.temp_budget_increase, self.temp_budget_expiry) {
            (Some(amount), Some(expiry)) if expiry.utc() > now => amount,
            _ => Decimal::ZERO,
        }
    }
    pub fn effective_max_budget(&self, now: DateTime<Utc>) -> Result<Option<Decimal>, BudgetError> {
        let Some(budget) = self.max_budget else {
            return Ok(None);
        };
        budget
            .checked_add(self.active_temp_budget_increase(now))
            .ok_or(BudgetError::Overflow)
            .map(Some)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionList(pub Option<Vec<String>>);
impl Default for PermissionList {
    fn default() -> Self {
        Self(Some(Vec::new()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMObjectPermissionTable {
    pub object_permission_id: String,
    #[serde(default)]
    pub mcp_servers: PermissionList,
    #[serde(default)]
    pub mcp_access_groups: PermissionList,
    #[serde(default)]
    pub mcp_tool_permissions: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub vector_stores: PermissionList,
    #[serde(default)]
    pub agents: PermissionList,
    #[serde(default)]
    pub agent_access_groups: PermissionList,
    #[serde(default)]
    pub models: PermissionList,
    #[serde(default)]
    pub mcp_toolsets: Option<Vec<String>>,
    #[serde(default)]
    pub blocked_tools: PermissionList,
    #[serde(default)]
    pub search_tools: PermissionList,
    #[serde(default)]
    pub mcp_tool_search_enabled: Option<bool>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
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
#[serde(transparent)]
pub struct ProjectedUserEmail(pub JsonData);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "OrganizationMembershipInput")]
pub struct OrganizationMembership {
    pub user_id: String,
    pub organization_id: String,
    #[serde(default)]
    pub user_role: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(default)]
    pub budget_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub user: Option<JsonData>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    pub user_email: Option<ProjectedUserEmail>,
}

#[derive(Debug, Deserialize)]
struct OrganizationMembershipInput {
    user_id: String,
    organization_id: String,
    #[serde(default)]
    user_role: Option<String>,
    #[serde(default)]
    spend: Decimal,
    #[serde(default)]
    budget_id: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
    #[serde(default)]
    user: Option<JsonData>,
    #[serde(default)]
    litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    user_email: Option<String>,
}

impl From<OrganizationMembershipInput> for OrganizationMembership {
    fn from(input: OrganizationMembershipInput) -> Self {
        let projected = input
            .user_email
            .map(|email| ProjectedUserEmail(JsonData::String(email)));
        let projected_email = projected.or_else(|| match input.user.as_ref() {
            Some(JsonData::Object(user)) => user.get("user_email").cloned().map(ProjectedUserEmail),
            _ => None,
        });
        Self {
            user_id: input.user_id,
            organization_id: input.organization_id,
            user_role: input.user_role,
            spend: input.spend,
            budget_id: input.budget_id,
            created_at: input.created_at,
            updated_at: input.updated_at,
            user: input.user,
            litellm_budget_table: input.litellm_budget_table,
            user_email: projected_email,
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
    #[serde(rename = "budget")]
    Budget(LiteLLMBudgetTable),
    #[serde(rename = "full")]
    Full(LiteLLMBudgetTableFull),
    #[serde(rename = "permission")]
    Permission(LiteLLMObjectPermissionTable),
    #[serde(rename = "membership")]
    Membership(OrganizationMembership),
}

pub fn decode_budget_permission_rows(
    bytes: &[u8],
) -> Result<BudgetPermissionRow, BudgetPermissionDecodeError> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err(BudgetPermissionDecodeError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(BudgetPermissionDecodeError::InvalidUtf8)?;
    check_depth(text)?;
    serde_json::from_str(text).map_err(BudgetPermissionDecodeError::Json)
}

#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}

fn check_depth(text: &str) -> Result<(), BudgetPermissionDecodeError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        match (state, byte) {
            (ScanState::Structural, b'{' | b'[') => {
                depth = depth.saturating_add(1);
                if usize::from(depth) > MAX_JSON_DEPTH {
                    return Err(BudgetPermissionDecodeError::TooDeep);
                }
            }
            (ScanState::Structural, b'}' | b']') => depth = depth.saturating_sub(1),
            (ScanState::Structural, b'"') | (ScanState::Escaped, _) => state = ScanState::Quoted,
            (ScanState::Quoted, b'\\') => state = ScanState::Escaped,
            (ScanState::Quoted, b'"') => state = ScanState::Structural,
            (ScanState::Structural | ScanState::Quoted, _) => {}
        }
    }
    Ok(())
}

#[path = "budget_permission_rows/tests.rs"]
mod tests;
