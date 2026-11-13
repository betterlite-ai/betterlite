use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use thiserror::Error;

mod tests;

pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;
pub const REDACTED: &str = "[REDACTED]";

const MATCH_FOUND: &str = "MATCH_FOUND";
const SCANNED_CONTENT_KEYS: [&str; 4] = [
    "text",
    "sanitizedText",
    "findings",
    "maliciousUriMatchedItems",
];
const SANITIZATION_RESULT: &str = "sanitizationResult";
const FILTER_RESULTS: &str = "filterResults";
const SDP_RESULT: &str = "sdpFilterResult";
const INSPECT_RESULT: &str = "inspectResult";
const DEIDENTIFY_RESULT: &str = "deidentifyResult";

// LiteLLM declares the sanitize response as Mapping[str, object], so this tree is its typed form.
pub type ArmorResponse = GuardrailValue;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct ModelArmorAPIError {
    pub detail: String,
}

fn field<'value>(value: &'value ArmorResponse, key: &str) -> Option<&'value ArmorResponse> {
    match value {
        ArmorResponse::Object(fields) => fields.get(key),
        ArmorResponse::Null
        | ArmorResponse::Boolean(_)
        | ArmorResponse::SignedInteger(_)
        | ArmorResponse::UnsignedInteger(_)
        | ArmorResponse::Decimal(_)
        | ArmorResponse::Text(_)
        | ArmorResponse::Array(_) => None,
    }
}

// LiteLLM compares the match state text exactly, case included.
fn matched(filter_entry: &ArmorResponse, filter_key: &str) -> bool {
    let Some(result) = field(filter_entry, filter_key) else {
        return false;
    };
    matches!(field(result, "matchState"), Some(ArmorResponse::Text(state)) if state == MATCH_FOUND)
}

fn filter_result_items(response: &ArmorResponse) -> Vec<&ArmorResponse> {
    let Some(sanitization) = field(response, SANITIZATION_RESULT) else {
        return Vec::new();
    };
    match field(sanitization, FILTER_RESULTS) {
        Some(ArmorResponse::Object(named)) => named.values().collect(),
        Some(ArmorResponse::Array(entries)) => entries.iter().collect(),
        Some(
            ArmorResponse::Null
            | ArmorResponse::Boolean(_)
            | ArmorResponse::SignedInteger(_)
            | ArmorResponse::UnsignedInteger(_)
            | ArmorResponse::Decimal(_)
            | ArmorResponse::Text(_),
        )
        | None => Vec::new(),
    }
}

#[must_use]
pub fn should_block_content(response: &ArmorResponse, allow_sanitization: bool) -> bool {
    filter_result_items(response)
        .into_iter()
        .any(|filter_entry| {
            if matched(filter_entry, "raiFilterResult")
                || matched(filter_entry, "piAndJailbreakFilterResult")
                || matched(filter_entry, "maliciousUriFilterResult")
                || matched(filter_entry, "csamFilterFilterResult")
                || matched(filter_entry, "virusScanFilterResult")
            {
                return true;
            }
            let Some(sdp) = field(filter_entry, SDP_RESULT) else {
                return false;
            };
            matched(sdp, INSPECT_RESULT) || (!allow_sanitization && matched(sdp, DEIDENTIFY_RESULT))
        })
}

#[must_use]
pub fn get_sanitized_content(response: &ArmorResponse) -> Option<String> {
    let filters = filter_result_items(response);
    for filter_entry in &filters {
        let Some(sdp) = field(filter_entry, SDP_RESULT) else {
            continue;
        };
        if !matched(sdp, DEIDENTIFY_RESULT) {
            continue;
        }
        let sanitized = field(sdp, DEIDENTIFY_RESULT)
            .and_then(|deidentify| field(deidentify, "data"))
            .and_then(|data| field(data, "text"));
        if let Some(ArmorResponse::Text(text)) = sanitized
            && !text.is_empty()
        {
            return Some(text.clone());
        }
    }
    for filter_entry in &filters {
        let Some(sdp) = field(filter_entry, SDP_RESULT) else {
            continue;
        };
        if matched(sdp, INSPECT_RESULT) {
            return None;
        }
    }
    root_sanitized_text(response)
}

fn root_sanitized_text(response: &ArmorResponse) -> Option<String> {
    if let Some(ArmorResponse::Text(text)) = field(response, "sanitizedText")
        && !text.is_empty()
    {
        return Some(text.clone());
    }
    match field(response, "text") {
        Some(ArmorResponse::Text(text)) if !text.is_empty() => Some(text.clone()),
        Some(
            ArmorResponse::Null
            | ArmorResponse::Boolean(_)
            | ArmorResponse::SignedInteger(_)
            | ArmorResponse::UnsignedInteger(_)
            | ArmorResponse::Decimal(_)
            | ArmorResponse::Text(_)
            | ArmorResponse::Array(_)
            | ArmorResponse::Object(_),
        )
        | None => None,
    }
}

#[must_use]
pub fn has_deidentify_match(response: &ArmorResponse) -> bool {
    filter_result_items(response)
        .into_iter()
        .any(|filter_entry| {
            field(filter_entry, SDP_RESULT).is_some_and(|sdp| matched(sdp, DEIDENTIFY_RESULT))
        })
}

#[must_use]
pub fn append_armor_response(
    existing: Option<ArmorResponse>,
    response: ArmorResponse,
) -> ArmorResponse {
    match existing {
        None | Some(ArmorResponse::Null) => response,
        Some(ArmorResponse::Array(mut responses)) => {
            responses.push(response);
            ArmorResponse::Array(responses)
        }
        Some(
            single @ (ArmorResponse::Boolean(_)
            | ArmorResponse::SignedInteger(_)
            | ArmorResponse::UnsignedInteger(_)
            | ArmorResponse::Decimal(_)
            | ArmorResponse::Text(_)
            | ArmorResponse::Object(_)),
        ) => ArmorResponse::Array(vec![single, response]),
    }
}

#[must_use]
pub fn redact_scanned_content(payload: &ArmorResponse, depth: usize) -> ArmorResponse {
    if depth >= DEFAULT_MAX_RECURSE_DEPTH {
        return ArmorResponse::Text(REDACTED.to_owned());
    }
    match payload {
        ArmorResponse::Object(fields) => ArmorResponse::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    if SCANNED_CONTENT_KEYS.contains(&key.as_str()) {
                        return (key.clone(), ArmorResponse::Text(REDACTED.to_owned()));
                    }
                    (key.clone(), redact_scanned_content(value, depth + 1))
                })
                .collect(),
        ),
        ArmorResponse::Array(items) => ArmorResponse::Array(
            items
                .iter()
                .map(|item| redact_scanned_content(item, depth + 1))
                .collect(),
        ),
        ArmorResponse::Null
        | ArmorResponse::Boolean(_)
        | ArmorResponse::SignedInteger(_)
        | ArmorResponse::UnsignedInteger(_)
        | ArmorResponse::Decimal(_)
        | ArmorResponse::Text(_) => payload.clone(),
    }
}

#[must_use]
pub fn build_logging_response(
    response: &ArmorResponse,
    sanitize_error_detail: bool,
) -> ArmorResponse {
    if sanitize_error_detail {
        return redact_scanned_content(response, 0);
    }
    response.clone()
}

#[must_use]
pub fn build_block_error_detail(
    message: &str,
    response: &ArmorResponse,
    sanitize_error_detail: bool,
) -> IndexMap<String, GuardrailValue> {
    let mut detail = IndexMap::new();
    detail.insert("error".to_owned(), GuardrailValue::Text(message.to_owned()));
    if !sanitize_error_detail {
        detail.insert("model_armor_response".to_owned(), response.clone());
    }
    detail
}

#[must_use]
pub fn build_api_error_detail(
    status_code: u16,
    response_text: &str,
    sanitize_error_detail: bool,
) -> String {
    if sanitize_error_detail {
        format!("Model Armor API error (upstream {status_code})")
    } else {
        format!("Model Armor API error (upstream {status_code}): {response_text}")
    }
}

pub fn raise_if_fail_closed(
    error: ModelArmorAPIError,
    fail_on_error: bool,
) -> Result<(), ModelArmorAPIError> {
    if fail_on_error { Err(error) } else { Ok(()) }
}

#[must_use]
pub fn extract_content_from_response(response: &ArmorResponse) -> String {
    let Some(ArmorResponse::Array(choices)) = field(response, "choices") else {
        return String::new();
    };
    let mut content = String::new();
    for choice in choices {
        let Some(message) = field(choice, "message") else {
            continue;
        };
        if let Some(ArmorResponse::Text(text)) = field(message, "content") {
            content.push_str(text);
        }
        if let Some(function_call) = field(message, "function_call")
            && !matches!(function_call, ArmorResponse::Null)
        {
            push_json(&mut content, function_call);
        }
        if let Some(ArmorResponse::Array(tool_calls)) = field(message, "tool_calls") {
            for tool_call in tool_calls {
                push_json(&mut content, tool_call);
            }
        }
    }
    content
}

fn push_json(content: &mut String, value: &ArmorResponse) {
    if let Ok(encoded) = serde_json::to_string(value) {
        content.push_str(&encoded);
    }
}
