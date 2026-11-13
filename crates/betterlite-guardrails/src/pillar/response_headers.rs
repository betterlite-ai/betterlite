use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::json_header::{encode_json_for_header, json_text, percent_encode_value};
use super::truncate::truncate_evidence_payload;
use super::{MAX_PILLAR_HEADER_VALUE_BYTES, TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY};

fn value_text(value: &GuardrailValue) -> String {
    match value {
        GuardrailValue::Null => String::from("null"),
        GuardrailValue::Boolean(flag) => flag.to_string(),
        GuardrailValue::SignedInteger(number) => number.to_string(),
        GuardrailValue::UnsignedInteger(number) => number.to_string(),
        GuardrailValue::Decimal(number) => number.to_string(),
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Array(_) | GuardrailValue::Object(_) => json_text(value),
    }
}

// LiteLLM's build_pillar_response_headers: x-pillar-* values, and the store keeps both the truncated
// evidence and the header map the response writer trusts
#[must_use]
pub fn build_pillar_response_headers(
    metadata: &mut BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(flagged) = metadata.get("pillar_flagged") {
        headers.insert(
            "x-pillar-flagged".to_owned(),
            value_text(flagged).to_lowercase(),
        );
    }
    if let Some(scanners) = metadata.get("pillar_scanners") {
        headers.insert(
            "x-pillar-scanners".to_owned(),
            encode_json_for_header(scanners),
        );
    }
    if let Some(evidence) = metadata.get("pillar_evidence") {
        let truncated = truncate_evidence_payload(evidence, MAX_PILLAR_HEADER_VALUE_BYTES);
        let was_truncated = truncated.was_truncated;
        let encoded = truncated.encoded;
        metadata.insert("pillar_evidence".to_owned(), truncated.evidence);
        if was_truncated {
            metadata.insert(
                "pillar_evidence_truncated".to_owned(),
                GuardrailValue::Boolean(true),
            );
        }
        headers.insert("x-pillar-evidence".to_owned(), encoded);
    }
    if let Some(session) = metadata.get("pillar_session_id_response") {
        headers.insert(
            "x-pillar-session-id".to_owned(),
            percent_encode_value(&value_text(session)),
        );
    }
    if !headers.is_empty() {
        let stored = headers
            .iter()
            .map(|(name, value)| (name.clone(), GuardrailValue::Text(value.clone())))
            .collect();
        metadata.insert(
            "pillar_response_headers".to_owned(),
            GuardrailValue::Object(stored),
        );
        metadata.insert(
            TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY.to_owned(),
            GuardrailValue::Boolean(true),
        );
    }
    headers
}
