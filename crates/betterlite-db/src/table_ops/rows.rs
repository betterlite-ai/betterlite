use std::collections::BTreeMap;
use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::{JsonData, LiteLLMBudgetTableFull, Timestamp};
use crate::identity_rows::end_user::LiteLLMEndUserTable;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::{TokenDecodeError, decode_token_record};
use crate::postgres_store::diagnostics::DriverError;
use crate::resource_repository::config::{ConfigDecodeError, decode_config_json};
use crate::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use crate::team_rows::team::LiteLLMTeamTable;
use crate::user_repository::projection::{UserRecordError, decode_user_record};

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
    #[error("the stored config value is not valid JSON text")]
    Config(#[source] ConfigDecodeError),
    #[error("the stored row is not a JSON object")]
    NotAnObject,
}

pub trait RowDecoder: Sized {
    fn decode_row(text: &str) -> Result<Self, RowError>;
}

impl RowDecoder for LiteLLMUserTable {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        decode_user_record(text.as_bytes())
            .map_err(RowError::User)?
            .ok_or(RowError::NotAnObject)
    }
}

impl RowDecoder for LiteLLMTeamTable {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        let record = TeamRecord::parse(text.as_bytes()).map_err(RowError::Team)?;
        decode_team_record(record.as_ref())
            .map_err(RowError::Team)?
            .ok_or(RowError::NotAnObject)
    }
}

impl RowDecoder for LiteLLMVerificationToken {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        decode_token_record(text.as_bytes()).map_err(RowError::Key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigParamRow {
    pub param_name: String,
    pub param_value: JsonData,
}

// insert_data stores json.dumps(param), so a JSON text value becomes the value it encodes
impl RowDecoder for ConfigParamRow {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        let stored: JsonData = serde_json::from_str(text).map_err(RowError::Model)?;
        let JsonData::Object(mut fields) = stored else {
            return Err(RowError::NotAnObject);
        };
        let Some(JsonData::String(param_name)) = fields.remove("param_name") else {
            return Err(RowError::NotAnObject);
        };
        let param_value = match fields.remove("param_value") {
            Some(JsonData::String(raw)) => {
                decode_config_json(raw.as_bytes()).map_err(RowError::Config)?
            }
            Some(value) => value,
            None => return Err(RowError::NotAnObject),
        };
        Ok(Self {
            param_name,
            param_value,
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BudgetMap {
    Text(String),
    Object(BTreeMap<String, JsonData>),
}

impl BudgetMap {
    fn decode(self) -> Result<Option<BTreeMap<String, JsonData>>, serde_json::Error> {
        match self {
            Self::Text(raw) => serde_json::from_str(&raw),
            Self::Object(fields) => Ok(Some(fields)),
        }
    }
}

#[derive(Deserialize)]
struct BudgetRecord {
    model_max_budget: Option<BudgetMap>,
    #[serde(flatten)]
    row: LiteLLMBudgetTableFull,
}

impl RowDecoder for LiteLLMBudgetTableFull {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        let mut stored: BudgetRecord = serde_json::from_str(text).map_err(RowError::Model)?;
        stored.row.budget.model_max_budget = stored
            .model_max_budget
            .map(BudgetMap::decode)
            .transpose()
            .map_err(RowError::Model)?
            .flatten();
        Ok(stored.row)
    }
}

impl RowDecoder for LiteLLMEndUserTable {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        serde_json::from_str(text).map_err(RowError::Model)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

// the stored api_key is hashed before it reaches this row, and Debug prints no part of it
impl fmt::Debug for SpendLogRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpendLogRow")
            .field("request_id", &self.request_id)
            .field("api_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl RowDecoder for SpendLogRow {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        serde_json::from_str(text).map_err(RowError::Model)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNotificationRow {
    pub request_id: String,
    pub user_id: String,
    // the Prisma model declares models optional, so a stored null stays None
    #[serde(default)]
    pub models: Option<Vec<String>>,
    pub justification: String,
    pub status: String,
}

impl RowDecoder for UserNotificationRow {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        serde_json::from_str(text).map_err(RowError::Model)
    }
}

#[derive(Debug, Clone)]
pub struct UserWithAliases {
    pub user: LiteLLMUserTable,
    pub key_aliases: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UserAliasRow {
    #[serde(default)]
    pub key_aliases: Vec<Option<String>>,
}

impl RowDecoder for UserWithAliases {
    fn decode_row(text: &str) -> Result<Self, RowError> {
        let aliases: UserAliasRow = serde_json::from_str(text).map_err(RowError::Json)?;
        Ok(Self {
            user: LiteLLMUserTable::decode_row(text)?,
            key_aliases: aliases.key_aliases,
        })
    }
}
