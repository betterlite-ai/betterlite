#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::{
    CompetitorIntentDetection, ContentFilterDetection, PatternDetection,
};

use super::{GuardrailStatus, GuardrailTraceInput, log_guardrail_information};

#[test]
fn logs_first_competitor_intent_and_exception_response() {
    let detection = ContentFilterDetection::CompetitorIntent(CompetitorIntentDetection {
        r#type: "competitor_intent".to_owned(),
        intent: "compare".to_owned(),
        confidence: 0.8,
        action_hint: "log_only".to_owned(),
        entities: BTreeMap::new(),
        signals: Vec::new(),
        evidence: Vec::new(),
    });
    let mut records = Vec::new();
    let detail = log_guardrail_information(
        &mut records,
        &GuardrailTraceInput {
            guardrail_provider: "provider",
            guardrail_name: "name",
            guardrail_id: None,
            policy_template: Some("policy".to_owned()),
            detections: &[detection],
            status: GuardrailStatus::GuardrailFailedToRespond,
            start_time: 1.0,
            end_time: 3.0,
            duration: Some(2.0_f64),
            masked_entity_count: &BTreeMap::new(),
            patterns_checked: 2,
            exception_str: Some("failed"),
        },
    );
    assert_eq!(detail.confidence_score, Some(0.8_f64));
    let entry = records.first().expect("entry");
    assert_eq!(entry.guardrail_response, "failed");
}

#[test]
fn logs_success_detections_as_json() {
    let detections = [ContentFilterDetection::Pattern(PatternDetection {
        r#type: "pattern".to_owned(),
        pattern_name: "email".to_owned(),
        action: "MASK".to_owned(),
    })];
    let mut records = Vec::new();
    log_guardrail_information(
        &mut records,
        &GuardrailTraceInput {
            guardrail_provider: "provider",
            guardrail_name: "name",
            guardrail_id: None,
            policy_template: None,
            detections: &detections,
            status: GuardrailStatus::Success,
            start_time: 1.0,
            end_time: 2.0,
            duration: Some(1.0_f64),
            masked_entity_count: &BTreeMap::new(),
            patterns_checked: 1,
            exception_str: None,
        },
    );
    let entry = records.first().expect("entry");
    assert_eq!(
        entry.guardrail_response,
        r#"[{"type":"pattern","pattern_name":"email","action":"MASK"}]"#
    );
    serde_json::from_str::<Vec<PatternDetection>>(&entry.guardrail_response).unwrap();
}
