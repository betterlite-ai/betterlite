use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

use super::config::{NomaV2Settings, trimmed};
use super::{KEYS_DUPLICATING_SCAN_INPUTS, LOGGING_DUPLICATE_INPUT_KEYS};

pub struct ScanRequest<'request> {
    pub inputs: &'request GuardrailApiInputs,
    pub request_data: &'request BTreeMap<String, GuardrailValue>,
    pub input_type: GuardrailInputType,
    pub log_target: Option<&'request GuardrailLogTarget>,
    pub application_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanPayload<'request> {
    pub inputs: &'request GuardrailApiInputs,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub input_type: GuardrailInputType,
    pub monitor_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_name: Option<String>,
}

#[must_use]
pub fn build<'request>(
    request: &'request ScanRequest<'request>,
    settings: &NomaV2Settings,
) -> ScanPayload<'request> {
    ScanPayload {
        inputs: request.inputs,
        request_data: request_payload(request.request_data, request.log_target),
        input_type: request.input_type,
        monitor_mode: settings.monitor_mode,
        application_id: request.application_id.clone(),
        gateway_name: settings.gateway_name.clone(),
    }
}

// LiteLLM's _sanitize_payload_for_transport, run on the whole payload before the POST
#[must_use]
pub fn wire_payload(payload: &ScanPayload<'_>) -> BTreeMap<String, GuardrailValue> {
    let text = match serde_json::to_string(payload) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!("Noma v2 guardrail: payload serialization failed: {error}");
            String::new()
        }
    };
    sanitize_payload(&text, true)
}

// LiteLLM's safe_json_loads with default {}, then its dict and emptiness warnings
#[must_use]
pub fn sanitize_payload(text: &str, had_content: bool) -> BTreeMap<String, GuardrailValue> {
    match serde_json::from_str::<GuardrailValue>(text) {
        Ok(GuardrailValue::Object(fields)) => {
            if fields.is_empty() && had_content {
                tracing::warn!(
                    "Noma v2 guardrail: payload serialization failed, falling back to empty payload"
                );
            }
            fields
        }
        Ok(other) => {
            tracing::warn!(
                "Noma v2 guardrail: payload sanitization produced non-object output (type={}), falling back to empty payload",
                value_type(&other)
            );
            BTreeMap::new()
        }
        Err(_not_json) => {
            if had_content {
                tracing::warn!(
                    "Noma v2 guardrail: payload serialization failed, falling back to empty payload"
                );
            }
            BTreeMap::new()
        }
    }
}

// LiteLLM resolves dynamic params, then the configured id, then the key alias for dashboard traceability
#[must_use]
pub fn application_id(
    dynamic: &BTreeMap<String, GuardrailValue>,
    configured: Option<&str>,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<String> {
    text_value(dynamic.get("application_id"))
        .or_else(|| configured.and_then(trimmed))
        .or_else(|| alias(request_data, "litellm_metadata"))
        .or_else(|| alias(request_data, "metadata"))
}

fn request_payload(
    request_data: &BTreeMap<String, GuardrailValue>,
    log_target: Option<&GuardrailLogTarget>,
) -> BTreeMap<String, GuardrailValue> {
    let filtered: BTreeMap<String, GuardrailValue> = request_data
        .iter()
        .filter(|(key, _value)| !KEYS_DUPLICATING_SCAN_INPUTS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut sanitized = match serde_json::to_string(&filtered) {
        Ok(text) => sanitize_payload(&text, !filtered.is_empty()),
        Err(_not_json) => BTreeMap::new(),
    };
    if let Some(target) = log_target {
        sanitized.insert("litellm_logging_obj".to_owned(), logging_details(target));
    }
    sanitized
}

fn logging_details(target: &GuardrailLogTarget) -> GuardrailValue {
    let Some(params) = &target.model_call_params else {
        return GuardrailValue::Null;
    };
    GuardrailValue::Object(
        params
            .iter()
            .filter(|(key, _value)| !LOGGING_DUPLICATE_INPUT_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn alias(request_data: &BTreeMap<String, GuardrailValue>, location: &str) -> Option<String> {
    let Some(GuardrailValue::Object(metadata)) = request_data.get(location) else {
        return None;
    };
    text_value(metadata.get("user_api_key_alias"))
}

fn text_value(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) => trimmed(text),
        Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        )
        | None => None,
    }
}

const fn value_type(value: &GuardrailValue) -> &'static str {
    match value {
        GuardrailValue::Null => "null",
        GuardrailValue::Boolean(_) => "bool",
        GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_) => "number",
        GuardrailValue::Text(_) => "string",
        GuardrailValue::Array(_) => "list",
        GuardrailValue::Object(_) => "object",
    }
}
