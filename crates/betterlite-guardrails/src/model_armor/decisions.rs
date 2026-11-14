use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use thiserror::Error;

use crate::model_armor::decisions::wire::{
    FilterEntry, FilterResults, MatchState, SanitizeResponse,
};

pub mod model_response;
mod tests;
pub mod wire;

pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;
pub const REDACTED: &str = "[REDACTED]";

const SCANNED_CONTENT_KEYS: [&str; 4] = [
    "text",
    "sanitizedText",
    "findings",
    "maliciousUriMatchedItems",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{detail}")]
pub struct ModelArmorAPIError {
    pub detail: String,
}

#[must_use]
pub fn filter_result_items(response: &SanitizeResponse) -> Vec<&FilterEntry> {
    match &response.sanitization_result.filter_results {
        FilterResults::Named(named) => named.values().collect(),
        FilterResults::Listed(entries) => entries.iter().collect(),
    }
}

#[must_use]
pub fn should_block_content(response: &SanitizeResponse, allow_sanitization: bool) -> bool {
    filter_result_items(response)
        .into_iter()
        .any(|filter_entry| {
            let sdp = &filter_entry.sdp;
            [
                &filter_entry.rai,
                &filter_entry.pi_and_jailbreak,
                &filter_entry.malicious_uri,
                &filter_entry.csam,
                &filter_entry.virus_scan,
                &sdp.inspect_result,
            ]
            .into_iter()
            .any(|result| result.match_state == MatchState::MatchFound)
                || (!allow_sanitization
                    && sdp.deidentify_result.match_state == MatchState::MatchFound)
        })
}

#[must_use]
pub fn get_sanitized_content(response: &SanitizeResponse) -> Option<String> {
    let filters = filter_result_items(response);
    let deidentified = filters
        .iter()
        .map(|filter_entry| &filter_entry.sdp.deidentify_result)
        .find(|deidentify| {
            deidentify.match_state == MatchState::MatchFound && !deidentify.data.text.is_empty()
        });
    if let Some(deidentify) = deidentified {
        return Some(deidentify.data.text.clone());
    }
    if filters
        .iter()
        .any(|filter_entry| filter_entry.sdp.inspect_result.match_state == MatchState::MatchFound)
    {
        return None;
    }
    [&response.sanitized_text, &response.text]
        .into_iter()
        .find(|text| !text.is_empty())
        .cloned()
}

#[must_use]
pub fn has_deidentify_match(response: &SanitizeResponse) -> bool {
    filter_result_items(response)
        .into_iter()
        .any(|filter_entry| {
            filter_entry.sdp.deidentify_result.match_state == MatchState::MatchFound
        })
}

#[must_use]
pub fn append_armor_response(
    existing: Option<GuardrailValue>,
    response: GuardrailValue,
) -> GuardrailValue {
    match existing {
        None | Some(GuardrailValue::Null) => response,
        Some(GuardrailValue::Array(mut responses)) => {
            responses.push(response);
            GuardrailValue::Array(responses)
        }
        Some(
            single @ (GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Object(_)),
        ) => GuardrailValue::Array(vec![single, response]),
    }
}

#[must_use]
pub fn redact_scanned_content(payload: &GuardrailValue, depth: usize) -> GuardrailValue {
    if depth >= DEFAULT_MAX_RECURSE_DEPTH {
        return GuardrailValue::Text(REDACTED.to_owned());
    }
    match payload {
        GuardrailValue::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    if SCANNED_CONTENT_KEYS.contains(&key.as_str()) {
                        return (key.clone(), GuardrailValue::Text(REDACTED.to_owned()));
                    }
                    (
                        key.clone(),
                        redact_scanned_content(value, depth.saturating_add(1)),
                    )
                })
                .collect(),
        ),
        GuardrailValue::Array(items) => GuardrailValue::Array(
            items
                .iter()
                .map(|item| redact_scanned_content(item, depth.saturating_add(1)))
                .collect(),
        ),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => payload.clone(),
    }
}

#[must_use]
pub fn build_logging_response(
    response: &GuardrailValue,
    sanitize_error_detail: bool,
) -> GuardrailValue {
    if sanitize_error_detail {
        return redact_scanned_content(response, 0);
    }
    response.clone()
}

#[must_use]
pub fn build_block_error_detail(
    message: &str,
    response: &GuardrailValue,
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
