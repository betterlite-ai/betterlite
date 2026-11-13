#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::response_headers::build_pillar_response_headers;
use super::{MAX_PILLAR_HEADER_VALUE_BYTES, TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY};

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn store(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn an_empty_store_produces_no_headers_and_no_side_effects() {
    let mut metadata = BTreeMap::new();
    let headers = build_pillar_response_headers(&mut metadata);
    assert!(headers.is_empty());
    assert!(metadata.is_empty());
}

#[test]
fn the_flagged_header_is_the_lowercase_word() {
    let mut metadata = store(&[("pillar_flagged", GuardrailValue::Boolean(true))]);
    let headers = build_pillar_response_headers(&mut metadata);
    assert_eq!(headers.get("x-pillar-flagged"), Some(&"true".to_owned()));
}

#[test]
fn a_text_flagged_value_is_lowercased() {
    let mut metadata = store(&[("pillar_flagged", text("TRUE"))]);
    let headers = build_pillar_response_headers(&mut metadata);
    assert_eq!(headers.get("x-pillar-flagged"), Some(&"true".to_owned()));
}

#[test]
fn scanners_are_percent_encoded_json() {
    let scanners =
        GuardrailValue::Object([("scanner".to_owned(), text("pii"))].into_iter().collect());
    let mut metadata = store(&[("pillar_scanners", scanners)]);
    let headers = build_pillar_response_headers(&mut metadata);
    assert_eq!(
        headers.get("x-pillar-scanners"),
        Some(&"%7B%22scanner%22%3A%22pii%22%7D".to_owned())
    );
}

#[test]
fn a_small_evidence_payload_is_only_encoded() {
    let evidence = GuardrailValue::Array(vec![GuardrailValue::Object(
        [("evidence".to_owned(), text("abc"))].into_iter().collect(),
    )]);
    let mut metadata = store(&[("pillar_evidence", evidence.clone())]);
    let headers = build_pillar_response_headers(&mut metadata);
    assert!(headers.contains_key("x-pillar-evidence"));
    assert_eq!(metadata.get("pillar_evidence"), Some(&evidence));
    assert!(!metadata.contains_key("pillar_evidence_truncated"));
}

#[test]
fn an_oversized_evidence_payload_is_truncated_and_marked() {
    let long = "x".repeat(9_000);
    let evidence = GuardrailValue::Array(vec![GuardrailValue::Object(
        [("evidence".to_owned(), text(&long))].into_iter().collect(),
    )]);
    let mut metadata = store(&[("pillar_evidence", evidence)]);
    let headers = build_pillar_response_headers(&mut metadata);
    let encoded = headers
        .get("x-pillar-evidence")
        .expect("the evidence header");
    assert!(encoded.len() <= MAX_PILLAR_HEADER_VALUE_BYTES);
    assert_eq!(
        metadata.get("pillar_evidence_truncated"),
        Some(&GuardrailValue::Boolean(true))
    );
    let Some(GuardrailValue::Array(kept)) = metadata.get("pillar_evidence") else {
        panic!("the evidence stays a list");
    };
    let Some(GuardrailValue::Object(entry)) = kept.first() else {
        panic!("the entry stays an object");
    };
    assert_eq!(
        entry.get("evidence_truncated"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn the_session_id_is_url_quoted() {
    let mut metadata = store(&[("pillar_session_id_response", text("a/b c"))]);
    let headers = build_pillar_response_headers(&mut metadata);
    assert_eq!(
        headers.get("x-pillar-session-id"),
        Some(&"a%2Fb%20c".to_owned())
    );
}

#[test]
fn the_store_keeps_the_header_map_and_the_trust_marker() {
    let mut metadata = store(&[("pillar_flagged", GuardrailValue::Boolean(false))]);
    let headers = build_pillar_response_headers(&mut metadata);
    let Some(GuardrailValue::Object(stored)) = metadata.get("pillar_response_headers") else {
        panic!("the header map is stored");
    };
    assert_eq!(stored.get("x-pillar-flagged"), Some(&text("false")));
    assert_eq!(headers.get("x-pillar-flagged"), Some(&"false".to_owned()));
    assert_eq!(
        metadata.get(TRUSTED_PILLAR_RESPONSE_HEADERS_METADATA_KEY),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn the_session_header_uses_the_response_version_not_the_persisted_one() {
    let mut metadata = store(&[
        ("pillar_session_id", text("sent")),
        ("pillar_session_id_response", text("returned")),
    ]);
    let headers = build_pillar_response_headers(&mut metadata);
    assert_eq!(
        headers.get("x-pillar-session-id"),
        Some(&"returned".to_owned())
    );
    assert_eq!(metadata.get("pillar_session_id"), Some(&text("sent")));
}
