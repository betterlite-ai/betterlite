use std::collections::BTreeMap;

use betterlite_config::sections::grayswan::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use crate::guardrail_base::inputs::GuardrailLogTarget;

use super::super::wire::{GraySwanDynamicParams, GraySwanPayload};
use super::Object;
use super::headers::extract_inbound_headers;

pub struct PreparePayloadRequest<'call> {
    pub messages: Vec<GuardrailValue>,
    pub tools: Option<Vec<GuardrailValue>>,
    pub dynamic: &'call GraySwanDynamicParams,
    pub request_data: &'call Object,
    pub log_target: Option<&'call GuardrailLogTarget>,
}

// LiteLLM's _prepare_payload: the dynamic body wins over the configured value, empty values are omitted
#[must_use]
pub fn prepare_payload(
    settings: &Configured,
    request: PreparePayloadRequest<'_>,
) -> GraySwanPayload {
    let PreparePayloadRequest {
        messages,
        tools,
        dynamic,
        request_data,
        log_target,
    } = request;
    GraySwanPayload {
        messages,
        tools: tools.filter(|tools| !tools.is_empty()),
        categories: dynamic
            .categories
            .clone()
            .filter(|categories| !categories.is_empty())
            .or_else(|| (!settings.categories.is_empty()).then(|| settings.categories.clone())),
        policy_id: dynamic
            .policy_id
            .clone()
            .filter(|policy_id| !policy_id.is_empty())
            .or_else(|| (!settings.policy_id.is_empty()).then(|| settings.policy_id.clone())),
        reasoning_mode: dynamic.reasoning_mode.or(settings.reasoning_mode),
        metadata: dynamic.metadata.clone(),
        litellm_metadata: litellm_metadata(request_data, log_target),
    }
}

fn litellm_metadata(
    request_data: &Object,
    log_target: Option<&GuardrailLogTarget>,
) -> Option<GuardrailValue> {
    let mut cleaned = match request_data.get("litellm_metadata") {
        Some(GuardrailValue::Object(fields)) => fields.clone(),
        _ => BTreeMap::new(),
    };
    if let Some(inbound) = extract_inbound_headers(request_data, log_target) {
        with_inbound_headers(&mut cleaned, inbound);
    }
    let sanitized = sanitize(&cleaned);
    (!sanitized.is_empty()).then_some(GuardrailValue::Object(sanitized))
}

// LiteLLM merges the inbound headers under their own key, and the inbound row wins on a duplicate
fn with_inbound_headers(
    cleaned: &mut BTreeMap<String, GuardrailValue>,
    inbound: BTreeMap<String, String>,
) {
    let mut headers = header_rows(inbound);
    if let Some(GuardrailValue::Object(existing)) = cleaned.remove("headers") {
        let mut merged = existing;
        merged.append(&mut headers);
        headers = merged;
    }
    cleaned.insert("headers".to_owned(), GuardrailValue::Object(headers));
}

fn header_rows(inbound: BTreeMap<String, String>) -> BTreeMap<String, GuardrailValue> {
    inbound
        .into_iter()
        .map(|(key, text)| (key, GuardrailValue::Text(text)))
        .collect()
}

// LiteLLM's safe_json_loads(safe_dumps(...), default={}): NUL bytes go, a value over the depth limit becomes its marker
fn sanitize(metadata: &BTreeMap<String, GuardrailValue>) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(&safe_dumps(metadata, DEFAULT_MAX_RECURSE_DEPTH, None).unwrap_or_default())
        .unwrap_or_default()
}
