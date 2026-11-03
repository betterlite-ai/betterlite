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
