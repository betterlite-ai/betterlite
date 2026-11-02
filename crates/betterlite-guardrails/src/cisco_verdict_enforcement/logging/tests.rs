#![cfg(test)]

use crate::cisco_configuration::records::CiscoVerdict;
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::logging::{insert_verdict_log_fields, rule_violations};

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn empty_verdict() -> CiscoVerdict {
    CiscoVerdict {
        is_safe: None,
        classifications: Vec::new(),
        severity: None,
        rules: Vec::new(),
        explanation: None,
        event_id: None,
        action: Some(Action::Allow),
        sanitized_text: None,
        sanitized_messages: None,
        sanitized_mcp_arguments: None,
    }
}

#[test]
fn rule_violations_excludes_none_violation_and_unnamed_rules() {
    let rules = vec![
        obj(r#"{"rule_name": "pii", "classification": "VIOLATION"}"#),
        obj(r#"{"rule_name": "clean", "classification": "NONE_VIOLATION"}"#),
        obj(r#"{"classification": "VIOLATION"}"#),
    ];
    assert_eq!(rule_violations(&rules), vec!["pii".to_owned()]);
}

#[test]
fn rule_violations_dedupes_and_sorts_names() {
    let rules = vec![
        obj(r#"{"rule_name": "zebra", "classification": "VIOLATION"}"#),
        obj(r#"{"rule_name": "alpha", "classification": "VIOLATION"}"#),
        obj(r#"{"rule_name": "alpha", "classification": "VIOLATION"}"#),
    ];
    assert_eq!(
        rule_violations(&rules),
        vec!["alpha".to_owned(), "zebra".to_owned()]
    );
}

#[test]
fn insert_verdict_log_fields_keeps_is_safe_false() {
    let mut verdict = empty_verdict();
    verdict.is_safe = Some(false);
    let mut fields = Fields::new();
    insert_verdict_log_fields(&mut fields, &verdict);
    assert_eq!(fields.get("is_safe").map(|raw| raw.get()), Some("false"));
}

#[test]
fn insert_verdict_log_fields_drops_empty_optional_fields() {
    let verdict = empty_verdict();
    let mut fields = Fields::new();
    insert_verdict_log_fields(&mut fields, &verdict);
    assert!(!fields.contains_key("is_safe"));
    assert!(!fields.contains_key("severity"));
    assert!(!fields.contains_key("classifications"));
    assert!(!fields.contains_key("rule_violations"));
    assert!(!fields.contains_key("event_id"));
}

#[test]
fn insert_verdict_log_fields_includes_populated_fields() {
    let mut verdict = empty_verdict();
    verdict.severity = Some("high".to_owned());
    verdict.classifications = vec!["PII".to_owned()];
    verdict.event_id = Some("evt-1".to_owned());
    verdict.rules = vec![obj(
        r#"{"rule_name": "pii", "classification": "VIOLATION"}"#,
    )];
    let mut fields = Fields::new();
    insert_verdict_log_fields(&mut fields, &verdict);
    assert_eq!(
        fields.get("severity").map(|raw| raw.get()),
        Some("\"high\"")
    );
    assert!(fields.contains_key("classifications"));
    assert!(fields.contains_key("rule_violations"));
    assert_eq!(
        fields.get("event_id").map(|raw| raw.get()),
        Some("\"evt-1\"")
    );
}
