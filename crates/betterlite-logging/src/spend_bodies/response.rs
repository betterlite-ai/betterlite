use betterlite_models::decisions::Json;

use super::SpendStoragePolicy;
use super::redaction::{redact_response, should_redact_message_logging};
use super::support::{Fields, as_object, counts_as_present, safe_structure, sanitize_fields};
use crate::python_json;
use crate::spend_sanitize::truncate::{TRUNCATED_PAYLOAD_FIELD, TRUNCATION_DB_SAFEGUARD_NOTE};

pub struct ResponseKwargs<'data> {
    pub realtime_tool_calls: Option<&'data Json>,
    pub litellm_params: Option<&'data Json>,
    pub dynamic_params: Option<&'data Json>,
}

#[must_use]
pub fn storage_response(
    payload: Option<&Json>,
    kwargs: Option<&ResponseKwargs<'_>>,
    policy: SpendStoragePolicy,
) -> String {
    let Some(stored_payload) = payload.and_then(as_object) else {
        return "{}".to_owned();
    };
    if !policy.sanitize.store_bodies {
        return "{}".to_owned();
    }
    let Some(original) = stored_payload.get("response") else {
        return "{}".to_owned();
    };
    if matches!(original, Json::Null) {
        return "{}".to_owned();
    }
    let stored = prepared_response(original, kwargs, policy);
    let wrapper = Fields::from([("response".to_owned(), stored)]);
    let sanitized = sanitize_fields(&wrapper, policy.sanitize.max_string_length);
    let Some(fields) = as_object(&sanitized) else {
        return "{}".to_owned();
    };
    let Some(sanitized_response) = fields.get("response") else {
        return "{}".to_owned();
    };
    if matches!(sanitized_response, Json::Null) {
        return "{}".to_owned();
    }
    let text = match sanitized_response {
        Json::Text(text) => text.replace('\0', ""),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            python_json::dumps(&safe_structure(sanitized_response, 0))
        }
    };
    if text.contains(TRUNCATED_PAYLOAD_FIELD) {
        tracing::info!(
            "Spend Log: response was truncated before storing in DB. {}",
            TRUNCATION_DB_SAFEGUARD_NOTE
        );
    }
    text
}

fn prepared_response(
    original: &Json,
    kwargs: Option<&ResponseKwargs<'_>>,
    policy: SpendStoragePolicy,
) -> Json {
    let mut stored = original.clone();
    if let Some(tool_calls) = kwargs.and_then(|response_kwargs| response_kwargs.realtime_tool_calls)
        && counts_as_present(tool_calls)
        && let Json::Object(fields) = &mut stored
    {
        fields.insert("tool_calls".to_owned(), tool_calls.clone());
    }
    let redact = kwargs.is_some_and(|response_kwargs| {
        should_redact_message_logging(
            response_kwargs.litellm_params.and_then(as_object),
            response_kwargs.dynamic_params.and_then(as_object),
            policy.turn_off_message_logging,
        )
    });
    if redact {
        return redact_response(stored);
    }
    stored
}
