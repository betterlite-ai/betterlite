#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::datadog_llm_privacy::router_span::router_span_fields;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

const DECISION: &str = r#"{
  "tier": "low",
  "cause": "heuristic",
  "score": 0.5,
  "signals": ["a long prompt"],
  "escalated": false,
  "context_escalated": true,
  "routed_model": "gpt-4o-mini",
  "tier_label": "Low"
}"#;

#[test]
fn router_span_fields_map_the_decision_onto_span_names() {
    let fields = router_span_fields(Some(&json(DECISION)), false);
    assert_eq!(fields.get("router_tier"), Some(&json("\"low\"")));
    assert_eq!(fields.get("router_cause"), Some(&json("\"heuristic\"")));
    assert_eq!(fields.get("router_score"), Some(&json("0.5")));
    assert_eq!(
        fields.get("router_signals"),
        Some(&json(r#"["a long prompt"]"#))
    );
    assert_eq!(fields.get("routed_model"), Some(&json("\"gpt-4o-mini\"")));
    assert_eq!(fields.get("tier_label"), None);
}

#[test]
fn router_span_fields_take_escalation_from_either_flag() {
    let fields = router_span_fields(Some(&json(DECISION)), false);
    assert_eq!(fields.get("router_escalated"), Some(&json("true")));
    let alone = router_span_fields(Some(&json(r#"{"escalated": false}"#)), false);
    assert_eq!(alone.get("router_escalated"), Some(&json("false")));
}

#[test]
fn router_span_fields_drop_the_prompt_quoting_signals_when_redacting() {
    let fields = router_span_fields(Some(&json(DECISION)), true);
    assert_eq!(fields.get("router_signals"), None);
    assert_eq!(fields.get("router_tier"), Some(&json("\"low\"")));
    assert_eq!(fields.get("router_escalated"), Some(&json("true")));
}

#[test]
fn router_span_fields_skip_a_null_value_and_an_unmapped_field() {
    let decision = json(r#"{"tier": null, "score": 0, "classifier_crux": "quote"}"#);
    let fields = router_span_fields(Some(&decision), false);
    assert_eq!(fields.get("router_tier"), None);
    assert_eq!(fields.get("router_score"), Some(&json("0")));
    assert_eq!(fields.get("router_escalated"), Some(&json("false")));
    assert_eq!(fields.len(), 2);
}

#[test]
fn router_span_fields_are_empty_without_a_decision_mapping() {
    assert!(router_span_fields(None, false).is_empty());
    assert!(router_span_fields(Some(&Json::Null), false).is_empty());
    assert!(router_span_fields(Some(&json("[]")), false).is_empty());
    assert!(router_span_fields(Some(&json("{}")), false).is_empty());
}
