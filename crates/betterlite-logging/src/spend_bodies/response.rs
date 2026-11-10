use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::SpendStoragePolicy;
use super::redaction::{redact_response, should_redact_message_logging};
use super::support::{NodeAllocator, as_mapping, counts_as_present, safe_structure, spend_value};
use crate::python_json;
use crate::spend_sanitize::projection::convert_to_json_serializable;
use crate::spend_sanitize::request_body::sanitize_request_body;
use crate::spend_sanitize::truncate::{TRUNCATED_PAYLOAD_FIELD, TRUNCATION_DB_SAFEGUARD_NOTE};
use crate::spend_sanitize::value::{SpendMapping, SpendValue};

pub struct ResponseKwargs<'data> {
    pub realtime_tool_calls: Option<&'data SpendValue>,
    pub litellm_params: Option<&'data SpendValue>,
    pub dynamic_params: Option<&'data SpendValue>,
}

#[must_use]
pub fn storage_response(
    payload: Option<&SpendMapping>,
    kwargs: Option<&ResponseKwargs<'_>>,
    policy: SpendStoragePolicy,
    nodes: &mut NodeAllocator,
) -> String {
    let Some(stored_payload) = payload else {
        return "{}".to_owned();
    };
    if !policy.sanitize.store_bodies {
        return "{}".to_owned();
    }
    let Some(original) = stored_payload.entries.get("response") else {
        return "{}".to_owned();
    };
    if matches!(original, SpendValue::Null) {
        return "{}".to_owned();
    }
    let overlaid = with_realtime_tool_calls(original, kwargs);
    let response_value = apply_redaction(overlaid, kwargs, policy, nodes);
    let wrapper = SpendMapping {
        node: nodes.allocate(),
        entries: IndexMap::from([("response".to_owned(), response_value)]),
    };
    let sanitized = sanitize_request_body(&wrapper, policy.sanitize.max_string_length);
    let Json::Object(fields) = &sanitized else {
        return "{}".to_owned();
    };
    let Some(sanitized_response) = fields.get("response") else {
        return "{}".to_owned();
    };
    if matches!(sanitized_response, Json::Null) {
        return "{}".to_owned();
    }
    let text = stored_response_text(sanitized_response);
    if text.contains(TRUNCATED_PAYLOAD_FIELD) {
        tracing::info!(
            "Spend Log: response was truncated before storing in DB. {}",
            TRUNCATION_DB_SAFEGUARD_NOTE
        );
    }
    text
}

fn with_realtime_tool_calls(
    stored: &SpendValue,
    kwargs: Option<&ResponseKwargs<'_>>,
) -> SpendValue {
    let mut response = stored.clone();
    if let Some(tool_calls) = kwargs.and_then(|response_kwargs| response_kwargs.realtime_tool_calls)
        && counts_as_present(tool_calls)
        && let SpendValue::Mapping(mapping) = &mut response
    {
        mapping
            .entries
            .insert("tool_calls".to_owned(), tool_calls.clone());
    }
    response
}

fn apply_redaction(
    stored: SpendValue,
    kwargs: Option<&ResponseKwargs<'_>>,
    policy: SpendStoragePolicy,
    nodes: &mut NodeAllocator,
) -> SpendValue {
    let redact = kwargs.is_some_and(|response_kwargs| {
        should_redact_message_logging(
            response_kwargs.litellm_params.and_then(as_mapping),
            response_kwargs.dynamic_params,
            policy.turn_off_message_logging,
        )
    });
    if !redact {
        return stored;
    }
    let json = convert_to_json_serializable(&stored);
    let redacted = redact_response(json);
    spend_value(&redacted, nodes)
}

fn stored_response_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.replace('\0', ""),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            python_json::dumps(&safe_structure(value, 0))
        }
    }
}
