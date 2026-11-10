use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::SpendStoragePolicy;
use super::classifier::{classifier_audit_fields, without_classifier_audit};
use super::redaction::{redact_request_body, should_redact_message_logging};
use super::support::{NodeAllocator, as_mapping, counts_as_present, safe_structure, spend_value};
use crate::python_json;
use crate::spend_sanitize::projection::convert_to_json_serializable;
use crate::spend_sanitize::request_body::sanitize_request_body;
use crate::spend_sanitize::truncate::{TRUNCATED_PAYLOAD_FIELD, TRUNCATION_DB_SAFEGUARD_NOTE};
use crate::spend_sanitize::value::{SpendMapping, SpendValue};

pub struct RequestKwargs<'data> {
    pub standard_logging_object: Option<&'data SpendMapping>,
    pub realtime_tools: Option<&'data SpendValue>,
    pub dynamic_params: Option<&'data SpendValue>,
}

#[must_use]
pub fn storage_request_body(
    litellm_params: &SpendMapping,
    kwargs: Option<&RequestKwargs<'_>>,
    policy: SpendStoragePolicy,
    nodes: &mut NodeAllocator,
) -> String {
    if !policy.sanitize.store_bodies {
        return "{}".to_owned();
    }
    let Some(mut body) = stored_body(litellm_params, nodes) else {
        return "{}".to_owned();
    };
    overlay_classifier_audit(&mut body, kwargs, nodes);
    overlay_realtime_tools(&mut body, kwargs);
    let sanitized = sanitize_stored_body(&body, litellm_params, kwargs, policy, nodes);
    let text = python_json::dumps(&safe_structure(&sanitized, 0));
    if text.contains(TRUNCATED_PAYLOAD_FIELD) {
        tracing::info!(
            "Spend Log: request body was truncated before storing in DB. {}",
            TRUNCATION_DB_SAFEGUARD_NOTE
        );
    }
    text
}

fn stored_body(litellm_params: &SpendMapping, nodes: &mut NodeAllocator) -> Option<SpendMapping> {
    let raw_proxy = litellm_params.entries.get("proxy_server_request")?;
    let proxy = as_mapping(raw_proxy)?;
    Some(match proxy.entries.get("body").and_then(as_mapping) {
        Some(body) if !body.entries.is_empty() => body.clone(),
        _ => SpendMapping {
            node: nodes.allocate(),
            entries: IndexMap::new(),
        },
    })
}

fn sanitize_stored_body(
    body: &SpendMapping,
    litellm_params: &SpendMapping,
    kwargs: Option<&RequestKwargs<'_>>,
    policy: SpendStoragePolicy,
    nodes: &mut NodeAllocator,
) -> Json {
    let redact = kwargs.is_some_and(|request_kwargs| {
        should_redact_message_logging(
            Some(litellm_params),
            request_kwargs.dynamic_params,
            policy.turn_off_message_logging,
        )
    });
    if !redact {
        return sanitize_request_body(body, policy.sanitize.max_string_length);
    }
    let cleaned = without_classifier_audit(body, nodes);
    let mut json = convert_to_json_serializable(&SpendValue::Mapping(cleaned));
    redact_request_body(&mut json);
    let rebuilt = spend_value(&json, nodes);
    as_mapping(&rebuilt).map_or_else(
        || Json::Object(IndexMap::new()),
        |mapping| sanitize_request_body(mapping, policy.sanitize.max_string_length),
    )
}

fn overlay_classifier_audit(
    body: &mut SpendMapping,
    kwargs: Option<&RequestKwargs<'_>>,
    nodes: &mut NodeAllocator,
) {
    let Some(payload) = kwargs.and_then(|request_kwargs| request_kwargs.standard_logging_object)
    else {
        return;
    };
    for (key, value) in classifier_audit_fields(payload, nodes) {
        body.entries.insert(key, value);
    }
}

fn overlay_realtime_tools(body: &mut SpendMapping, kwargs: Option<&RequestKwargs<'_>>) {
    let Some(tools) = kwargs.and_then(|request_kwargs| request_kwargs.realtime_tools) else {
        return;
    };
    if counts_as_present(tools) {
        body.entries.insert("tools".to_owned(), tools.clone());
    }
}
