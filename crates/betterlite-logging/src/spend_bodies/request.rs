use super::SpendStoragePolicy;
use super::classifier::classifier_audit_fields;
use super::redaction::{redact_request_body, should_redact_message_logging};
use super::support::{
    Fields, as_object, counts_as_present, log_truncation, safe_structure, sanitize_fields,
};
use crate::python_json;
use betterlite_models::decisions::Json;
pub struct RequestKwargs<'data> {
    pub standard_logging_object: Option<&'data Json>,
    pub realtime_tools: Option<&'data Json>,
    pub dynamic_params: Option<&'data Json>,
}
#[must_use]
pub fn storage_request_body(
    litellm_params: &Json,
    kwargs: Option<&RequestKwargs<'_>>,
    policy: SpendStoragePolicy,
) -> String {
    let Some(params) = as_object(litellm_params).filter(|_| policy.sanitize.store_bodies) else {
        return "{}".to_owned();
    };
    let Some(mut body) = body_fields(params, kwargs) else {
        return "{}".to_owned();
    };
    let redact = kwargs.is_some_and(|request_kwargs| {
        should_redact_message_logging(
            Some(params),
            request_kwargs.dynamic_params.and_then(as_object),
            policy.turn_off_message_logging,
        )
    });
    let sanitized = if redact {
        body.shift_remove("classifier_input");
        body.shift_remove("originating_request_masked");
        let mut json = Json::Object(body);
        redact_request_body(&mut json);
        let Json::Object(fields) = json else {
            return "{}".to_owned();
        };
        sanitize_fields(&fields, policy.sanitize.max_string_length)
    } else {
        sanitize_fields(&body, policy.sanitize.max_string_length)
    };
    let text = python_json::dumps(&safe_structure(&sanitized, 0));
    log_truncation(&text, "request body");
    text
}
fn body_fields(params: &Fields, kwargs: Option<&RequestKwargs<'_>>) -> Option<Fields> {
    let proxy = params.get("proxy_server_request");
    if proxy.is_some_and(|value| !matches!(value, Json::Object(_))) {
        return None;
    }
    let mut body = proxy
        .and_then(as_object)
        .and_then(|fields| fields.get("body"))
        .and_then(as_object)
        .filter(|body| !body.is_empty())
        .cloned()
        .unwrap_or_default();
    if let Some(request_kwargs) = kwargs {
        if let Some(standard) = request_kwargs.standard_logging_object.and_then(as_object) {
            body.extend(classifier_audit_fields(standard));
        }
        if let Some(tools) = request_kwargs.realtime_tools
            && counts_as_present(tools)
        {
            body.insert("tools".to_owned(), tools.clone());
        }
    }
    Some(body)
}
