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

#[rstest::rstest]
#[case::blocked("guardrail_intervened", true)]
#[case::failed("guardrail_failed_to_respond", true)]
#[case::passed("success", false)]
fn guardrail_error_statuses(#[case] status: &str, #[case] is_error: bool) {
    let entry = BTreeMap::from([
        (
            "guardrail_status".to_owned(),
            OtelModelValue::String(status.to_owned()),
        ),
        (
            "guardrail_action".to_owned(),
            OtelModelValue::String("action".to_owned()),
        ),
    ]);
    let data = GuardrailSpanData::from_logging_entry(&entry);
    assert_eq!(data.guardrail_name, "guardrail");
    assert_eq!(data.error.is_some(), is_error);
    if let Some(error) = data.error {
        assert_eq!(error.error_type.as_deref(), Some(status));
        assert_eq!(error.message.as_deref(), Some("action"));
    }
}

#[test]
fn guardrail_captures_canonical_fields() {
    let entry = serde_json::from_str(r#"{"guardrail_name":"pii","guardrail_mode":["pre_call","post_call"],"masked_entity_count":{"email":2},"guardrail_provider":"provider","guardrail_action":"mask","guardrail_response":"verdict","violation_categories":["pii"],"confidence_score":0.5,"risk_score":0.25,"duration":2,"start_time":100,"end_time":102,"guardrail_id":"id","policy_template":"policy","detection_method":"regex","guardrail_usage":{"units":3},"guardrail_cost":0.01,"guardrail_cost_in_spend":false}"#).unwrap();
    let data = GuardrailSpanData::from_logging_entry(&entry);
    assert_eq!(data.guardrail_name, "pii");
    assert_eq!(
        [
            data.mode.as_deref(),
            data.provider.as_deref(),
            data.action.as_deref(),
            data.response_json.as_deref(),
            data.guardrail_id.as_deref(),
            data.policy_template.as_deref(),
            data.detection_method.as_deref(),
            data.usage_json.as_deref()
        ],
        [
            Some("pre_call,post_call"),
            Some("provider"),
            Some("mask"),
            Some("verdict"),
            Some("id"),
            Some("policy"),
            Some("regex"),
            Some(r#"{"units":3}"#)
        ]
    );
    assert_eq!(data.masked_entity_count, Some(2));
    assert_eq!(data.violation_categories, ["pii"]);
    assert_eq!(data.cost, Some(Decimal::new(1, 2)));
    assert_eq!(data.cost_in_spend, Some(false));
    assert!(data.error.is_none());
    assert!(
        GuardrailSpanData::from_logging_entry(&BTreeMap::new())
            .cost_in_spend
            .is_none()
    );
}

#[test]
fn guardrail_preserves_scores_and_execution_window() {
    let entry = serde_json::from_str(r#"{"confidence_score":0.5,"risk_score":0.25,"duration":2,"start_time":100,"end_time":102}"#).unwrap();
    let data = GuardrailSpanData::from_logging_entry(&entry);
    assert_eq!(
        [
            data.confidence_score,
            data.risk_score,
            data.duration,
            data.start_time,
            data.end_time
        ]
        .map(|value| value.map(f64::to_bits)),
        [0.5_f64, 0.25_f64, 2.0_f64, 100.0_f64, 102.0_f64].map(|value| Some(value.to_bits()))
    );
}
