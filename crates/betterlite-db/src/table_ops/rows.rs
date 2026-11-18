use std::error::Error;

use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::{JsonData, Timestamp};
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::TokenDecodeError;
use crate::postgres_store::diagnostics::DriverError;
use crate::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use crate::team_rows::team::LiteLLMTeamTable;
use crate::user_repository::projection::{UserRecordError, decode_user_record};

use super::TableOpsError;

#[derive(Debug, thiserror::Error)]
pub enum RowError {
    #[error("the stored row column is unavailable")]
    Column(#[source] DriverError),
    #[error("the stored row is invalid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the stored key row cannot be decoded")]
    Key(#[source] TokenDecodeError),
    #[error("the stored user row cannot be decoded")]
    User(#[source] UserRecordError),
    #[error("the stored team row cannot be decoded")]
    Team(#[source] TeamRecordError),
    #[error("the stored row does not match its model")]
    Model(#[source] serde_json::Error),
    #[error("the stored row is not a JSON object")]
    NotAnObject,
}

pub fn decode_user_row(text: &str) -> Result<LiteLLMUserTable, RowError> {
    match decode_user_record(text.as_bytes()).map_err(RowError::User)? {
        Some(user) => Ok(user),
        None => Err(RowError::NotAnObject),
    }
}

pub fn decode_team_row(text: &str) -> Result<LiteLLMTeamTable, RowError> {
    let record = TeamRecord::parse(text.as_bytes()).map_err(RowError::Team)?;
    match decode_team_record(record.as_ref()).map_err(RowError::Team)? {
        Some(team) => Ok(team),
        None => Err(RowError::NotAnObject),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLogRow {
    pub request_id: String,
    pub call_type: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::float::serialize",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(rename = "startTime")]
    pub start_time: Timestamp,
    #[serde(rename = "endTime")]
    pub end_time: Timestamp,
    #[serde(default)]
    pub request_duration_ms: Option<i64>,
    #[serde(rename = "completionStartTime", default)]
    pub completion_start_time: Option<Timestamp>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub metadata: Option<JsonData>,
    #[serde(default)]
    pub cache_hit: Option<String>,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub request_tags: Option<JsonData>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub end_user: Option<String>,
    #[serde(default)]
    pub requester_ip_address: Option<String>,
    #[serde(default)]
    pub messages: Option<JsonData>,
    #[serde(default)]
    pub response: Option<JsonData>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(default)]
    pub billing_agent_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub proxy_server_request: Option<JsonData>,
    #[serde(default)]
    pub litellm_call_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNotificationRow {
    pub request_id: String,
    pub user_id: String,
    #[serde(default)]
    pub models: Vec<String>,
    pub justification: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserWithAliases {
    pub user: LiteLLMUserTable,
    pub key_aliases: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UserAliasRow {
    #[serde(default)]
    pub key_aliases: Vec<Option<String>>,
}

pub fn decode_user_with_aliases(text: &str) -> Result<UserWithAliases, RowError> {
    let aliases: UserAliasRow = serde_json::from_str(text).map_err(RowError::Json)?;
    Ok(UserWithAliases {
        user: decode_user_row(text)?,
        key_aliases: aliases.key_aliases,
    })
}

pub fn decode_model_rows<Row, ClientError>(
    rows: &[String],
) -> Result<Vec<Row>, TableOpsError<ClientError>>
where
    Row: DeserializeOwned,
    ClientError: Error + Send + Sync + 'static,
{
    rows.iter()
        .map(|text| {
            serde_json::from_str(text)
                .map_err(RowError::Model)
                .map_err(Into::into)
        })
        .collect()
}
