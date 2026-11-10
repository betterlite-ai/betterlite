use super::SpendStoragePolicy;
use super::redaction::{redact_response, should_redact_message_logging};
use super::support::{
    Fields, as_object, counts_as_present, log_truncation, safe_structure, sanitize_fields,
};
use crate::python_json;
use betterlite_models::decisions::Json;
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
    let Some(original) = stored_response(payload, policy) else {
        return "{}".to_owned();
    };
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
    let redacted = if redact {
        redact_response(stored)
    } else {
        stored
    };
    let wrapper = Fields::from([("response".to_owned(), redacted)]);
    let sanitized = sanitize_fields(&wrapper, policy.sanitize.max_string_length);
    let Some(sanitized_response) = as_object(&sanitized)
        .and_then(|fields| fields.get("response"))
        .filter(|response| !matches!(response, Json::Null))
    else {
        return "{}".to_owned();
    };
    let text = if let Json::Text(text) = sanitized_response {
        text.replace('\0', "")
    } else {
        python_json::dumps(&safe_structure(sanitized_response, 0))
    };
    log_truncation(&text, "response");
    text
}
fn stored_response(payload: Option<&Json>, policy: SpendStoragePolicy) -> Option<&Json> {
    payload
        .filter(|_| policy.sanitize.store_bodies)
        .and_then(as_object)
        .and_then(|fields| fields.get("response"))
        .filter(|response| !matches!(response, Json::Null))
}
