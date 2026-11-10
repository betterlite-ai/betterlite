#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::datadog_llm_privacy::guardrail_redact::{redact_guardrails, redacted_records};
use crate::fields::payload::records::GuardrailInformation;

const REDACTED: &str = "REDACTED_BY_LITELM";

const RECORD_JSON: &str = r#"{
  "guardrail_name": "pii",
  "guardrail_provider": "presidio",
  "guardrail_mode": {"default": "pre_call"},
  "guardrail_status": "guardrail_flagged",
  "start_time": 1704067200.0,
  "end_time": 1704067200.25,
  "duration": 0.25,
  "masked_entity_count": {"EMAIL": 2},
  "guardrail_id": "g-1",
  "policy_template": "pii-v1",
  "detection_method": "regex",
  "confidence_score": 0.9,
  "classification": {"type": "pii"},
  "match_details": ["email"],
  "patterns_checked": 3,
  "alert_recipients": ["sec@example.com"],
  "risk_score": 0.5,
  "violation_categories": ["pii"],
  "guardrail_action": "mask",
  "guardrail_usage": {"scans": 1},
  "guardrail_cost": 0.01,
  "guardrail_cost_by_unit": {"scan": 0.01, "free": null},
  "guardrail_cost_in_spend": true,
  "guardrail_request": {"prompt": "secret"},
  "guardrail_response": {"flagged": true}
}"#;

fn record() -> GuardrailInformation {
    serde_json::from_str(RECORD_JSON).expect("the record fixture parses")
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

#[test]
fn guardrail_redaction_marks_prompt_fields_and_keeps_audit_fields() {
    let redacted = redact_guardrails(Some(&json(RECORD_JSON))).expect("the record redacts");
    assert_eq!(redacted.len(), 1);
    assert_eq!(redacted[0].get("guardrail_name"), Some(&json("\"pii\"")));
    assert_eq!(
        redacted[0].get("guardrail_request"),
        Some(&json(&format!("\"{REDACTED}\"")))
    );
    assert_eq!(
        redacted[0].get("match_details"),
        Some(&json("\"REDACTED_BY_LITELM\""))
    );
    assert_eq!(redacted[0].get("duration"), Some(&json("0.25")));
    assert_eq!(
        redacted[0].get("masked_entity_count"),
        Some(&json(r#"{"EMAIL": 2}"#))
    );
}

#[test]
fn guardrail_redaction_drops_a_key_neither_set_classifies() {
    let value = json(r#"{"guardrail_name": "pii", "custom_unknown_field": "leak"}"#);
    let redacted = redact_guardrails(Some(&value)).expect("the record redacts");
    assert_eq!(redacted[0].get("custom_unknown_field"), None);
    assert_eq!(redacted[0].len(), 1);
}

#[test]
fn guardrail_redaction_accepts_a_single_record_or_a_list() {
    let single = json(r#"{"guardrail_name": "one"}"#);
    assert_eq!(
        redact_guardrails(Some(&single)).expect("one record").len(),
        1
    );
    let listed = json(r#"[{"guardrail_name": "one"}, 5, {"guardrail_name": "two"}]"#);
    let entries = redact_guardrails(Some(&listed)).expect("the list redacts");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].get("guardrail_name"), Some(&json("\"two\"")));
}

#[test]
fn guardrail_redaction_reports_no_records_for_null_or_a_scalar() {
    assert!(redact_guardrails(None).is_none());
    assert_eq!(
        redact_guardrails(Some(&Json::Null)),
        None,
        "a JSON null is no record list"
    );
    let entries =
        redact_guardrails(Some(&json("\"not a record\""))).expect("a scalar is no record");
    assert!(entries.is_empty());
}

#[test]
fn redacting_a_typed_record_keeps_the_audit_fields_and_marks_the_prompt_ones() {
    let records = [record()];
    let redacted = redacted_records(&records).expect("the record encodes");
    assert_eq!(
        redacted[0].get("guardrail_request"),
        Some(&json("\"REDACTED_BY_LITELM\""))
    );
    assert_eq!(
        redacted[0].get("classification"),
        Some(&json("\"REDACTED_BY_LITELM\""))
    );
    assert_eq!(
        redacted[0].get("guardrail_cost_by_unit"),
        Some(&json(r#"{"scan": 0.01, "free": null}"#))
    );
}
