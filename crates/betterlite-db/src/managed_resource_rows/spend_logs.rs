use std::collections::BTreeMap;
use std::mem::take;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::budget_permission_rows::JsonData;

use super::ManagedRowsDecodeError;
use super::wire::RowTimestamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogPayload {
    Text(String),
    Items(Vec<JsonData>),
    Map(BTreeMap<String, JsonData>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendLog {
    pub request_id: String,
    #[serde(rename = "api_key")]
    pub hashed_api_key: String,
    #[serde(default = "default_model")]
    pub model: Option<String>,
    #[serde(default = "default_api_base")]
    pub api_base: Option<String>,
    pub call_type: String,
    #[serde(default = "default_spend", with = "rust_decimal::serde::float_option")]
    pub spend: Option<Decimal>,
    #[serde(default = "default_total_tokens")]
    pub total_tokens: Option<i64>,
    #[serde(default = "default_prompt_tokens")]
    pub prompt_tokens: Option<i64>,
    #[serde(default = "default_completion_tokens")]
    pub completion_tokens: Option<i64>,
    #[serde(rename = "startTime")]
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub start_time: Option<String>,
    #[serde(rename = "endTime")]
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub end_time: Option<String>,
    #[serde(default = "default_user")]
    pub user: Option<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Option<JsonData>,
    #[serde(default = "default_cache_hit")]
    pub cache_hit: Option<String>,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub request_tags: Option<JsonData>,
    #[serde(default)]
    pub requester_ip_address: Option<String>,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub messages: Option<LogPayload>,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub response: Option<LogPayload>,
    #[serde(default)]
    pub created_at: Option<RowTimestamp>,
    #[serde(default)]
    pub updated_at: Option<RowTimestamp>,
}

#[must_use]
pub const fn default_model() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_api_base() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_user() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub fn default_cache_hit() -> Option<String> {
    Some("False".to_owned())
}

#[must_use]
pub const fn default_metadata() -> Option<JsonData> {
    Some(JsonData::Object(BTreeMap::new()))
}

#[must_use]
pub const fn default_spend() -> Option<Decimal> {
    Some(Decimal::ZERO)
}

#[must_use]
pub const fn default_total_tokens() -> Option<i64> {
    Some(0)
}

#[must_use]
pub const fn default_prompt_tokens() -> Option<i64> {
    Some(0)
}

#[must_use]
pub const fn default_completion_tokens() -> Option<i64> {
    Some(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorLog {
    pub request_id: Option<String>,
    #[serde(default = "default_api_base")]
    pub api_base: Option<String>,
    #[serde(default = "default_model_group")]
    pub model_group: Option<String>,
    #[serde(default = "default_litellm_model_name")]
    pub litellm_model_name: Option<String>,
    #[serde(default = "default_model_id")]
    pub model_id: Option<String>,
    #[serde(default = "default_request_kwargs")]
    pub request_kwargs: Option<BTreeMap<String, JsonData>>,
    #[serde(default = "default_exception_type")]
    pub exception_type: Option<String>,
    #[serde(default = "default_status_code")]
    pub status_code: Option<String>,
    #[serde(default = "default_exception_string")]
    pub exception_string: Option<String>,
    #[serde(rename = "startTime")]
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub start_time: Option<String>,
    #[serde(rename = "endTime")]
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub end_time: Option<String>,
}

#[must_use]
pub const fn default_model_group() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_litellm_model_name() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_model_id() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_request_kwargs() -> Option<BTreeMap<String, JsonData>> {
    Some(BTreeMap::new())
}

#[must_use]
pub const fn default_exception_type() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_status_code() -> Option<String> {
    Some(String::new())
}

#[must_use]
pub const fn default_exception_string() -> Option<String> {
    Some(String::new())
}

pub(crate) fn decode_spend_log(row: &mut JsonData) -> Result<SpendLog, ManagedRowsDecodeError> {
    let fields = super::object_fields(row)?;
    decode_json_string(fields, "metadata")?;
    decode_json_string(fields, "request_tags")?;
    super::decode_row(row)
}

pub(crate) fn decode_error_log(
    row: &mut JsonData,
    request_id: Uuid,
) -> Result<ErrorLog, ManagedRowsDecodeError> {
    let fields = super::object_fields(row)?;
    if !fields.contains_key("request_id") {
        let _previous = fields.insert(
            "request_id".to_owned(),
            JsonData::String(request_id.to_string()),
        );
    }
    super::decode_row(row)
}

// pydantic's Json type parses a supplied string and rejects every other non-null shape
fn decode_json_string(
    fields: &mut BTreeMap<String, JsonData>,
    field: &'static str,
) -> Result<(), ManagedRowsDecodeError> {
    let Some(value) = fields.get_mut(field) else {
        return Ok(());
    };
    match value {
        JsonData::Null => Ok(()),
        JsonData::String(text) => {
            let raw = take(text);
            super::check_depth(&raw)?;
            let parsed = serde_json::from_str(&raw).map_err(ManagedRowsDecodeError::Json)?;
            *value = parsed;
            Ok(())
        }
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_) => {
            Err(ManagedRowsDecodeError::JsonFieldNotString { field })
        }
    }
}
