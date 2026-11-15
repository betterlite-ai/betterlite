use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use percent_encoding::utf8_percent_encode;

use super::json_header::{QUOTE_SAFE_NONE, json_text};
use super::truncate::truncate_evidence_payload;
use super::{MAX_PILLAR_HEADER_VALUE_BYTES, TRUSTED_PILLAR_HEADERS_KEY};

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

// LiteLLM replaces the stored evidence with the shortened form and marks that it shortened it
fn evidence_header(metadata: &mut BTreeMap<String, GuardrailValue>) -> Option<String> {
    let evidence = metadata.get("pillar_evidence")?;
    let truncated = truncate_evidence_payload(evidence, MAX_PILLAR_HEADER_VALUE_BYTES);
    metadata.insert("pillar_evidence".to_owned(), truncated.evidence);
    if truncated.was_truncated {
        metadata.insert(
            "pillar_evidence_truncated".to_owned(),
            GuardrailValue::Boolean(true),
        );
    }
    Some(truncated.encoded)
}

// LiteLLM's build_pillar_response_headers, plus the truncated evidence and header map its store keeps
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
        let encoded = utf8_percent_encode(&json_text(scanners), &QUOTE_SAFE_NONE).to_string();
        headers.insert("x-pillar-scanners".to_owned(), encoded);
    }
    if let Some(encoded) = evidence_header(metadata) {
        headers.insert("x-pillar-evidence".to_owned(), encoded);
    }
    if let Some(session) = metadata.get("pillar_session_id_response") {
        let encoded = utf8_percent_encode(&value_text(session), &QUOTE_SAFE_NONE).to_string();
        headers.insert("x-pillar-session-id".to_owned(), encoded);
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
            TRUSTED_PILLAR_HEADERS_KEY.to_owned(),
            GuardrailValue::Boolean(true),
        );
    }
    headers
}
