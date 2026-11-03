#![cfg(test)]

use super::*;
#[test]
fn guardrail_marks_intervention_as_error() {
    let entry = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"guardrail_status":"guardrail_intervened"}"#,
    )
    .unwrap();
    assert!(
        GuardrailSpanData::from_logging_entry(&entry)
            .error
            .is_some()
    );
}
#[test]
fn null_guardrail_response_and_usage_are_absent() {
    let absent = serde_json::from_str::<BTreeMap<String, OtelModelValue>>("{}").unwrap();
    let nulls = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"guardrail_response":null,"guardrail_usage":null}"#,
    )
    .unwrap();
    let missing = GuardrailSpanData::from_logging_entry(&absent);
    let explicit_null = GuardrailSpanData::from_logging_entry(&nulls);
    assert_eq!(missing.response_json, explicit_null.response_json);
    assert_eq!(missing.usage_json, explicit_null.usage_json);
}
