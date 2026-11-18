pub mod access_groups;
pub mod ocr;
pub mod projection;
pub mod records;
pub mod redaction;
pub mod router_metadata;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::logging_metadata::decode::DecodeError;

pub const BATCH_COST_REQUEST_ID_SUFFIX: &str = "_batch_cost";
pub const MAX_MODEL_NAME_LENGTH: usize = 256;
pub const MCP_SPEND_LOG_MODEL_PREFIX: &str = "MCP: ";

#[derive(Debug, thiserror::Error)]
pub enum SpendMetadataError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

pub(crate) const fn as_object(value: &Json) -> Option<&JsonObject> {
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

mod tests;
