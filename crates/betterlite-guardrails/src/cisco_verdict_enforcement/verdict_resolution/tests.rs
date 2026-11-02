#![cfg(test)]

use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::verdict_resolution::resolve_verdict;

#[test]
fn resolve_verdict_defaults_to_allow_when_action_is_missing() {
    let response = Json::parse(r#"{"is_safe": true}"#);
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.action, Some(Action::Allow));
}

#[test]
fn resolve_verdict_normalizes_an_unrecognized_action_string_to_block() {
    let response = Json::parse(r#"{"action": "quarantine"}"#);
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.action, Some(Action::Block));
}

#[test]
fn resolve_verdict_normalizes_a_known_action_string() {
    let response = Json::parse(r#"{"action": "sanitize"}"#);
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.action, Some(Action::Redact));
}

#[test]
fn resolve_verdict_reads_the_declared_scalar_fields() {
    let response = Json::parse(
        r#"{"is_safe": false, "severity": "high", "explanation": "why", "event_id": "evt-1"}"#,
    );
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.is_safe, Some(false));
    assert_eq!(verdict.severity, Some("high".to_owned()));
    assert_eq!(verdict.explanation, Some("why".to_owned()));
    assert_eq!(verdict.event_id, Some("evt-1".to_owned()));
}

#[test]
fn resolve_verdict_classifications_prefers_the_list_field_over_the_singular_one() {
    let response =
        Json::parse(r#"{"classifications": ["PII", "SECRET"], "classification": "OTHER"}"#);
    let verdict = resolve_verdict(&response);
    assert_eq!(
        verdict.classifications,
        vec!["PII".to_owned(), "SECRET".to_owned()]
    );
}

#[test]
fn resolve_verdict_classifications_falls_back_to_the_singular_field() {
    let response = Json::parse(r#"{"classification": "PII"}"#);
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.classifications, vec!["PII".to_owned()]);
}

#[test]
fn resolve_verdict_rules_keeps_only_object_entries() {
    let response = Json::parse(r#"{"rules": [{"rule_name": "pii"}, "not-an-object", 1]}"#);
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.rules.len(), 1);
}

#[test]
fn resolve_verdict_reads_sanitized_text_and_messages() {
    let response = Json::parse(
        r#"{"action": "redact", "sanitized_text": "clean", "sanitized_messages": [{"role": "user", "content": "clean"}]}"#,
    );
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.sanitized_text, Some("clean".to_owned()));
    assert_eq!(
        verdict.sanitized_messages.map(|messages| messages.len()),
        Some(1)
    );
}

#[test]
fn resolve_verdict_allows_when_the_envelope_is_not_an_object() {
    let response = Json::parse("[1, 2, 3]");
    let verdict = resolve_verdict(&response);
    assert_eq!(verdict.action, Some(Action::Allow));
    assert_eq!(verdict.is_safe, None);
    assert!(verdict.classifications.is_empty());
}
