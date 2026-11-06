use betterlite_models::decisions::Json;

use crate::spend_sanitize::REDACTED_BY_LITELM;
use crate::spend_sanitize::SpendSanitizePolicy;
use crate::spend_sanitize::guardrails::sanitize_guardrail_information;
use crate::spend_sanitize::value::SpendValue;

use super::support::{lookup, number, object, parse, value};

fn redacted() -> Json {
    Json::Text(REDACTED_BY_LITELM.to_owned())
}

fn policy(store_bodies: bool) -> SpendSanitizePolicy {
    SpendSanitizePolicy {
        store_bodies,
        max_string_length: 2048,
    }
}

fn redact(source: &str) -> Json {
    sanitize_guardrail_information(&value(source), &policy(false))
}

#[test]
fn redacts_only_the_present_prompt_carrying_fields() {
    let sanitized =
        redact(r#"{"guardrail_name":"pii","match_details":"user said hi","risk_score":3}"#);
    assert_eq!(lookup(&sanitized, "0.match_details"), &redacted());
    assert_eq!(
        lookup(&sanitized, "0.guardrail_name"),
        &Json::Text("pii".to_owned())
    );
    assert_eq!(lookup(&sanitized, "0.risk_score"), &number(3));
    assert!(
        object(lookup(&sanitized, "0"))
            .get("classification")
            .is_none()
    );
}

#[test]
fn normalizes_a_bare_mapping_to_a_single_entry_list() {
    let sanitized = redact(r#"{"guardrail_name":"pii","match_details":"secret"}"#);
    let Json::List(entries) = &sanitized else {
        panic!("a bare mapping becomes a list")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(lookup(&sanitized, "0.match_details"), &redacted());
}

#[test]
fn drops_non_mapping_entries_from_a_list() {
    let sanitized = redact(r#"["text", 3, null, {"guardrail_name":"keep"}]"#);
    let Json::List(entries) = &sanitized else {
        panic!("a list stays a list")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(
        lookup(&sanitized, "0.guardrail_name"),
        &Json::Text("keep".to_owned())
    );
}

#[test]
fn preserves_numeric_compression_stats_and_excludes_booleans() {
    let sanitized = redact(
        r#"[{"guardrail_response":{"tokens_before":10,"tokens_saved":2.5,"compression_ratio":0.5,"flag":true,"text":"secret"}}]"#,
    );
    let expected = parse(r#"{"tokens_before":10,"tokens_saved":2.5,"compression_ratio":0.5}"#);
    assert_eq!(lookup(&sanitized, "0.guardrail_response"), &expected);
    assert!(
        object(lookup(&sanitized, "0.guardrail_response"))
            .get("text")
            .is_none()
    );
}

#[test]
fn redacts_a_guardrail_response_without_numeric_stats() {
    let sanitized =
        redact(r#"[{"guardrail_response":{"text":"only"},"guardrail_response_text":"keep"}]"#);
    assert_eq!(lookup(&sanitized, "0.guardrail_response"), &redacted());
    assert_eq!(
        lookup(&sanitized, "0.guardrail_response_text"),
        &Json::Text("keep".to_owned())
    );
}

#[test]
fn keeps_the_source_shape_when_bodies_are_stored() {
    let source = value(r#"{"guardrail_response":{"text":"stored"},"match_details":"stored too"}"#);
    let sanitized = sanitize_guardrail_information(&source, &policy(true));
    let Json::Object(entries) = &sanitized else {
        panic!("a bare mapping stays a bare mapping")
    };
    assert_eq!(
        entries["match_details"],
        Json::Text("stored too".to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "guardrail_response.text"),
        &Json::Text("stored".to_owned())
    );
}

#[test]
fn yields_no_entries_for_values_outside_the_declared_list_or_mapping() {
    let sanitized =
        sanitize_guardrail_information(&SpendValue::Text("x".to_owned()), &policy(false));
    assert_eq!(sanitized, Json::List(Vec::new()));
    let absent = sanitize_guardrail_information(&SpendValue::Null, &policy(false));
    assert_eq!(absent, Json::List(Vec::new()));
}
