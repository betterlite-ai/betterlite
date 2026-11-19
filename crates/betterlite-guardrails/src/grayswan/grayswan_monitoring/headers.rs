use std::collections::BTreeMap;

use betterlite_config::sections::grayswan::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;

use crate::guardrail_base::inputs::GuardrailLogTarget;

use super::super::error::{GraySwanFailure, GraySwanGuardrailAPIError};
use super::Object;

// LiteLLM forwards only this inbound header to the monitoring call
pub const FORWARDED_HEADER_NAMES: [&str; 1] = ["shade_scan_id"];

pub fn prepare_headers(settings: &Configured) -> Result<HeaderMap, GraySwanGuardrailAPIError> {
    let api_key = settings.api_key.expose_secret();
    let mut headers = HeaderMap::new();
    let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|source| GraySwanFailure::InvalidHeader { source })?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut key_header = HeaderValue::from_str(api_key)
        .map_err(|source| GraySwanFailure::InvalidHeader { source })?;
    key_header.set_sensitive(true);
    headers.insert("grayswan-api-key", key_header);
    Ok(headers)
}

// LiteLLM's _extract_inbound_headers: the first non-empty headers dict of four places, filtered to shade_scan_id
#[must_use]
pub fn extract_inbound_headers(
    request_data: &Object,
    log_target: Option<&GuardrailLogTarget>,
) -> Option<BTreeMap<String, String>> {
    let rows = request_header_rows(request_data).or_else(|| {
        let params = log_target?.model_call_params.as_ref()?;
        nested_object(params, "metadata", "headers")
    })?;
    let mut forwarded = BTreeMap::new();
    for (key, value) in rows {
        if !is_forwarded_name(key) {
            continue;
        }
        if let GuardrailValue::Text(text) = value {
            forwarded.insert(key.clone(), text.clone());
        }
    }
    (!forwarded.is_empty()).then_some(forwarded)
}

fn request_header_rows(request_data: &Object) -> Option<&BTreeMap<String, GuardrailValue>> {
    [
        nested_object(request_data, "proxy_server_request", "headers"),
        request_data.get("headers").and_then(non_empty_object),
        nested_object(request_data, "metadata", "headers"),
    ]
    .into_iter()
    .flatten()
    .next()
}

fn nested_object<'data>(
    data: &'data Object,
    outer: &str,
    key: &str,
) -> Option<&'data BTreeMap<String, GuardrailValue>> {
    match data.get(outer) {
        Some(GuardrailValue::Object(fields)) => fields.get(key).and_then(non_empty_object),
        _ => None,
    }
}

fn non_empty_object(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    (!fields.is_empty()).then_some(fields)
}

fn is_forwarded_name(key: &str) -> bool {
    FORWARDED_HEADER_NAMES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(key))
}
