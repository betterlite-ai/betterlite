pub mod access_groups;
pub mod ocr;
pub mod projection;
pub mod records;
pub mod redaction;
pub mod router_metadata;

use betterlite_models::call_helpers::metadata::{
    CallParams, MetadataMap, litellm_metadata_from_params,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use rust_decimal::serde::arbitrary_precision_option;
use serde::Serialize;

use crate::logging_metadata::decode::{DecodeError, decode};

pub const BATCH_COST_REQUEST_ID_SUFFIX: &str = "_batch_cost";
const LITELLM_PARAMS_RECORD: &str = "litellm params";
const METADATA_BUCKETS: [&str; 2] = ["metadata", "litellm_metadata"];
pub const MAX_MODEL_NAME_LENGTH: usize = 256;
pub const MCP_SPEND_LOG_MODEL_PREFIX: &str = "MCP: ";

#[derive(Debug, thiserror::Error)]
pub enum SpendMetadataError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the {field} field must have the declared type")]
    FieldType { field: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpendLogStatus {
    Success,
    Failure,
}

impl SpendLogStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[must_use]
pub const fn as_object(value: &Json) -> Option<&JsonObject> {
    match value {
        Json::Object(entries) => Some(entries),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn as_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// A declared field: absent and null both fall to the caller's default, and any other type is named.
pub fn optional_field<'value, T, F>(
    value: Option<&'value Json>,
    field: &'static str,
    convert: F,
) -> Result<Option<T>, SpendMetadataError>
where
    F: FnOnce(&'value Json) -> Option<T>,
{
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(present) => convert(present)
            .map(Some)
            .ok_or(SpendMetadataError::FieldType { field }),
    }
}

// A declared string field: absent and null both fall to the caller's default, and any other type is named.
pub fn optional_text_field<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value str>, SpendMetadataError> {
    match value {
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Null) | None => Ok(None),
        Some(_) => Err(SpendMetadataError::FieldType { field }),
    }
}

pub(crate) fn nonempty_text(value: Option<&Json>) -> Option<&str> {
    if let Some(Json::Text(text)) = value
        && !text.is_empty()
    {
        return Some(text);
    }
    None
}

// get_spend_logs_id: the first nonempty string among the response id, the standard payload id and the call id.
#[must_use]
pub fn spend_logs_id(
    call_type: &str,
    response_obj: &JsonObject,
    kwargs: &JsonObject,
) -> Option<String> {
    let standard_logging_id = kwargs
        .get("standard_logging_object")
        .and_then(as_object)
        .and_then(|payload| nonempty_text(payload.get("id")));
    let candidates = [
        nonempty_text(response_obj.get("id")),
        standard_logging_id,
        nonempty_text(kwargs.get("litellm_call_id")),
    ];
    let resolved = candidates.into_iter().flatten().next()?;
    if call_type == "aretrieve_batch" {
        return Some(format!("{resolved}{BATCH_COST_REQUEST_ID_SUFFIX}"));
    }
    Some(resolved.to_owned())
}

// _get_status_for_spend_log: only the exact string failure is a failure.
#[must_use]
pub fn status_for_spend_log(metadata: &JsonObject) -> SpendLogStatus {
    if matches!(metadata.get("status"), Some(Json::Text(status)) if status == "failure") {
        return SpendLogStatus::Failure;
    }
    SpendLogStatus::Success
}

// _looks_like_model_name: one MCP prefix removed, at most 256 Python characters and no Python whitespace.
#[must_use]
pub fn looks_like_model_name(model: &str) -> bool {
    let candidate = model
        .strip_prefix(MCP_SPEND_LOG_MODEL_PREFIX)
        .unwrap_or(model);
    let has_space = candidate
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}'));
    candidate.chars().count() <= MAX_MODEL_NAME_LENGTH && !has_space
}

// SpendLogsPayload: one field per _types.py annotation, in declaration order.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SpendLogsPayload {
    pub request_id: String,
    pub call_type: String,
    pub api_key: String,
    #[serde(serialize_with = "arbitrary_precision_option::serialize")]
    pub spend: Option<Decimal>,
    pub total_tokens: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    #[serde(rename = "startTime")]
    pub start_time: String,
    #[serde(rename = "endTime")]
    pub end_time: String,
    #[serde(rename = "completionStartTime")]
    pub completion_start_time: Option<String>,
    pub model: String,
    pub model_id: String,
    pub model_group: String,
    pub mcp_namespaced_tool_name: Option<String>,
    pub billing_agent_id: Option<String>,
    pub agent_id: Option<String>,
    pub api_base: String,
    pub user: String,
    pub metadata: String,
    pub cache_hit: String,
    pub cache_key: String,
    pub request_tags: String,
    pub team_id: String,
    pub organization_id: String,
    pub end_user: String,
    pub requester_ip_address: Option<String>,
    pub custom_llm_provider: String,
    pub messages: String,
    pub response: String,
    pub proxy_server_request: String,
    pub session_id: Option<String>,
    pub request_duration_ms: Option<i64>,
    pub status: SpendLogStatus,
    pub litellm_call_id: Option<String>,
}

// get_litellm_metadata_from_kwargs: the merged metadata bucket of a litellm_params record.
pub fn merged_spend_metadata(
    litellm_params: &JsonObject,
) -> Result<MetadataMap, SpendMetadataError> {
    let mut buckets = JsonObject::new();
    for name in METADATA_BUCKETS {
        if let Some(value) = litellm_params.get(name) {
            buckets.insert(name.to_owned(), value.clone());
        }
    }
    let params: CallParams = decode(LITELLM_PARAMS_RECORD, buckets)?;
    Ok(litellm_metadata_from_params(Some(&params)))
}

mod tests;
