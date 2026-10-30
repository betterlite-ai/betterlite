use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};

const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(&value) {
        return Ok(timestamp.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
        .map(|timestamp| timestamp.and_utc())
        .map_err(D::Error::custom)
}

fn deserialize_optional_timestamp<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let timestamp = Option::<String>::deserialize(deserializer)?
        .map(|value| {
            if let Ok(timestamp) = DateTime::parse_from_rfc3339(&value) {
                return Ok(timestamp.with_timezone(&Utc));
            }
            NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|timestamp| timestamp.and_utc())
                .map_err(D::Error::custom)
        })
        .transpose();
    timestamp
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
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub temp_budget_expiry: Option<DateTime<Utc>>,
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
    #[serde(default, deserialize_with = "deserialize_optional_timestamp")]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "deserialize_timestamp")]
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
pub struct PermissionList(Option<Vec<String>>);
impl Default for PermissionList {
    fn default() -> Self {
        Self(Some(Vec::new()))
    }
}
impl PermissionList {}

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
pub struct ProjectedUserEmail(JsonData);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrganizationMembership {
    pub user_id: String,
    pub organization_id: String,
    #[serde(default)]
    pub user_role: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub updated_at: DateTime<Utc>,
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
    #[serde(deserialize_with = "deserialize_timestamp")]
    created_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_timestamp")]
    updated_at: DateTime<Utc>,
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
impl<'de> Deserialize<'de> for OrganizationMembership {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        OrganizationMembershipInput::deserialize(deserializer).map(Self::from)
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
    let value: JsonData = serde_json::from_str(text).map_err(BudgetPermissionDecodeError::Json)?;
    if json_data_depth(&value) > MAX_JSON_DEPTH {
        return Err(BudgetPermissionDecodeError::TooDeep);
    }
    serde_json::from_str(text).map_err(BudgetPermissionDecodeError::Json)
}
fn json_data_depth(value: &JsonData) -> usize {
    match value {
        JsonData::Array(values) => values
            .iter()
            .map(json_data_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        JsonData::Object(values) => values
            .values()
            .map(json_data_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) => 0,
    }
}

#[path = "budget_permission_rows/tests.rs"]
mod tests;
