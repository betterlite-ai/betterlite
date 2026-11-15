use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::HeaderMap;

use crate::guardrail_base::inputs::GuardrailLogTarget;

pub const REQUESTER_HEADER: &str = "hl-requester-id";
pub const PROJECT_HEADER: &str = "hl-project-id";
pub const ROUNDTRIP_HEADER: &str = "hl-roundtrip-id";
pub const RUNTIME_ACTION_HEADER: &str = "hl-runtime-action";
pub const DEFAULT_REQUESTER: &str = "LiteLLM";

fn object(value: Option<&GuardrailValue>) -> Option<&BTreeMap<String, GuardrailValue>> {
    match value {
        Some(GuardrailValue::Object(fields)) => Some(fields),
        _ => None,
    }
}

// headers reach the endpoint in whatever casing the client sent, and httpx looks them up case-insensitively
fn lowercase_texts(fields: &BTreeMap<String, GuardrailValue>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (name, value) in fields {
        if let GuardrailValue::Text(text) = value {
            headers.insert(name.to_lowercase(), text.clone());
        }
    }
    headers
}

#[must_use]
pub fn proxy_request_headers(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, String> {
    object(request_data.get("proxy_server_request"))
        .and_then(|proxy| object(proxy.get("headers")))
        .map_or_else(BTreeMap::new, lowercase_texts)
}

#[must_use]
pub fn logged_request_headers(
    logging_obj: Option<&GuardrailLogTarget>,
) -> BTreeMap<String, String> {
    logging_obj
        .and_then(|target| target.model_call_params.as_ref())
        .and_then(|params| object(params.get("metadata")))
        .and_then(|metadata| object(metadata.get("headers")))
        .map_or_else(BTreeMap::new, lowercase_texts)
}

// LiteLLM reads the logging object's headers only when the proxy request carried none
#[must_use]
pub fn request_headers(
    request_data: &BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&GuardrailLogTarget>,
) -> BTreeMap<String, String> {
    let headers = proxy_request_headers(request_data);
    if !headers.is_empty() {
        return headers;
    }
    logged_request_headers(logging_obj)
}

#[must_use]
pub fn evaluation_model(request_data: &BTreeMap<String, GuardrailValue>) -> String {
    match object(request_data.get("litellm_logging_obj")).and_then(|fields| fields.get("model")) {
        Some(GuardrailValue::Text(model)) if !model.is_empty() => model.clone(),
        _ => "unknown".to_owned(),
    }
}

#[must_use]
pub fn header_value(headers: &HeaderMap, name: &str, default: &str) -> String {
    match headers.get(name).and_then(|value| value.to_str().ok()) {
        Some(value) => value.to_owned(),
        None => default.to_owned(),
    }
}
