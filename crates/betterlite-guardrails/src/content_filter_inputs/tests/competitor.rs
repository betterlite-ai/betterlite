#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::content_filter_records::overrides::{
    CompetitorIntentEvidenceEntry, CompetitorIntentResult,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use crate::content_filter_inputs::competitor::{
    CompetitorMessageTemplates, DEFAULT_REFRAME_MESSAGE, DEFAULT_REFUSE_MESSAGE,
    apply_competitor_intent_policy,
};
use crate::guardrail_base::failure::GuardrailFailure;

use super::{base, with_logs};

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn competitor_result(action_hint: &str) -> CompetitorIntentResult {
    CompetitorIntentResult {
        intent: Some("competitor_comparison".to_owned()),
        confidence: Some(0.82_f64),
        entities: BTreeMap::from([
            ("brand_self".to_owned(), vec!["Acme".to_owned()]),
            ("competitors".to_owned(), vec!["Globex".to_owned()]),
        ]),
        signals: vec!["ranking".to_owned()],
        action_hint: Some(action_hint.to_owned()),
        evidence: vec![
            CompetitorIntentEvidenceEntry {
                r#type: Some("entity".to_owned()),
                key: Some("competitors".to_owned()),
                value: Some("Globex".to_owned()),
                r#match: None,
            },
            CompetitorIntentEvidenceEntry {
                r#type: Some("signal".to_owned()),
                key: Some("ranking".to_owned()),
                value: None,
                r#match: Some("better than".to_owned()),
            },
        ],
    }
}

fn templates<'templates>(
    refuse: Option<&'templates str>,
    reframe: Option<&'templates str>,
) -> CompetitorMessageTemplates<'templates> {
    CompetitorMessageTemplates { refuse, reframe }
}

fn request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("model".to_owned(), text("gpt-4o-mini")),
        ("messages".to_owned(), GuardrailValue::Array(Vec::new())),
    ])
}

#[test]
fn competitor_intent_detection() {
    let guardrail = base(None);
    let data = request_data();
    let mut detections = Vec::new();
    apply_competitor_intent_policy(
        &guardrail,
        &competitor_result("allow"),
        &templates(None, None),
        &data,
        &mut detections,
    )
    .expect("an allowed intent passes");
    assert_eq!(detections.len(), 1);
    let ContentFilterDetection::CompetitorIntent(detection) = &detections[0] else {
        panic!("the detection is a competitor intent");
    };
    assert_eq!(detection.r#type, "competitor_intent");
    assert_eq!(detection.intent, "competitor_comparison");
    assert_eq!(detection.confidence, 0.82_f64);
    assert_eq!(detection.action_hint, "allow");
    assert_eq!(detection.signals, ["ranking"]);
    assert_eq!(detection.entities["brand_self"], ["Acme"]);
    assert_eq!(detection.entities["competitors"], ["Globex"]);
    assert_eq!(detection.evidence[0].value.as_deref(), Some("Globex"));
    assert_eq!(detection.evidence[1].value, None);
    let mut logged = Vec::new();
    apply_competitor_intent_policy(
        &guardrail,
        &competitor_result("log_only"),
        &templates(None, None),
        &data,
        &mut logged,
    )
    .expect("a log only intent passes");
    assert_eq!(logged.len(), 1);
}

#[test]
fn competitor_refuse_uses_the_configured_template() {
    let mut detections = Vec::new();
    let error = apply_competitor_intent_policy(
        &base(None),
        &competitor_result("refuse"),
        &templates(Some("no comparisons here"), None),
        &request_data(),
        &mut detections,
    )
    .unwrap_err();
    assert_eq!(detections.len(), 1);
    let GuardrailFailure::HttpObject { status, detail } = error else {
        panic!("a refused intent raises the HTTP detail");
    };
    assert_eq!(status, 400);
    assert_eq!(detail.get("error"), Some(&text("no comparisons here")));
    assert_eq!(detail.get("intent"), Some(&text("competitor_comparison")));
    assert_eq!(
        detail.get("confidence"),
        Some(&GuardrailValue::Decimal(0.82_f64))
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn competitor_refuse_falls_back_to_the_default_message(#[case] configured: Option<&str>) {
    let error = apply_competitor_intent_policy(
        &base(None),
        &competitor_result("refuse"),
        &templates(configured, None),
        &request_data(),
        &mut Vec::new(),
    )
    .unwrap_err();
    let GuardrailFailure::HttpObject { detail, .. } = error else {
        panic!("a refused intent raises the HTTP detail");
    };
    assert_eq!(detail.get("error"), Some(&text(DEFAULT_REFUSE_MESSAGE)));
}

#[test]
fn competitor_reframe_builds_a_passthrough() {
    let data = request_data();
    let mut detections = Vec::new();
    let error = apply_competitor_intent_policy(
        &base(None),
        &competitor_result("reframe"),
        &templates(None, Some("compare specific features")),
        &data,
        &mut detections,
    )
    .unwrap_err();
    assert_eq!(detections.len(), 1);
    let GuardrailFailure::ModifyResponse(failure) = error else {
        panic!("a reframed intent passes the response through");
    };
    assert_eq!(failure.message, "compare specific features");
    assert_eq!(failure.model, "gpt-4o-mini");
    assert_eq!(
        failure.guardrail_name.as_deref(),
        Some("test-content-filter")
    );
    assert_eq!(failure.request_data, data);
    assert_eq!(failure.original_response, None);
    assert_reframe_info(&failure.detection_info);
}

fn assert_reframe_info(info: &BTreeMap<String, GuardrailValue>) {
    assert_eq!(info.get("intent"), Some(&text("competitor_comparison")));
    assert_eq!(
        info.get("confidence"),
        Some(&GuardrailValue::Decimal(0.82_f64))
    );
    assert_eq!(info.get("action_hint"), Some(&text("reframe")));
    assert_eq!(
        info.get("signals"),
        Some(&GuardrailValue::Array(vec![text("ranking")]))
    );
    assert_eq!(
        info.get("entities"),
        Some(&GuardrailValue::Object(BTreeMap::from([
            (
                "brand_self".to_owned(),
                GuardrailValue::Array(vec![text("Acme")])
            ),
            (
                "competitors".to_owned(),
                GuardrailValue::Array(vec![text("Globex")])
            ),
        ])))
    );
    assert_evidence(info);
}

fn assert_evidence(info: &BTreeMap<String, GuardrailValue>) {
    let Some(GuardrailValue::Array(evidence)) = info.get("evidence") else {
        panic!("the detection info carries the evidence list");
    };
    assert_eq!(evidence.len(), 2);
    let GuardrailValue::Object(entity) = &evidence[0] else {
        panic!("an evidence entry is an object");
    };
    assert_eq!(entity.get("type"), Some(&text("entity")));
    assert_eq!(entity.get("key"), Some(&text("competitors")));
    assert_eq!(entity.get("value"), Some(&text("Globex")));
    assert!(!entity.contains_key("match"));
    let GuardrailValue::Object(signal) = &evidence[1] else {
        panic!("an evidence entry is an object");
    };
    assert_eq!(signal.get("value"), None);
    assert_eq!(signal.get("match"), Some(&text("better than")));
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn competitor_reframe_falls_back_to_the_default_message(#[case] configured: Option<&str>) {
    let error = apply_competitor_intent_policy(
        &base(None),
        &competitor_result("reframe"),
        &templates(None, configured),
        &request_data(),
        &mut Vec::new(),
    )
    .unwrap_err();
    let GuardrailFailure::ModifyResponse(failure) = error else {
        panic!("a reframed intent passes the response through");
    };
    assert_eq!(failure.message, DEFAULT_REFRAME_MESSAGE);
}

#[test]
fn competitor_defaults_fill_missing_result_fields() {
    let result = CompetitorIntentResult {
        intent: None,
        confidence: None,
        entities: BTreeMap::new(),
        signals: Vec::new(),
        action_hint: None,
        evidence: Vec::new(),
    };
    let mut detections = Vec::new();
    apply_competitor_intent_policy(
        &base(None),
        &result,
        &templates(None, None),
        &request_data(),
        &mut detections,
    )
    .expect("an intent without fields continues");
    let ContentFilterDetection::CompetitorIntent(detection) = &detections[0] else {
        panic!("the detection is a competitor intent");
    };
    assert_eq!(detection.intent, "other");
    assert_eq!(detection.confidence, 0.0);
    assert_eq!(detection.action_hint, "allow");
}

#[test]
fn competitor_intent_logging_names_the_action() {
    let data = request_data();
    let (outcome, logs) = with_logs(|| {
        apply_competitor_intent_policy(
            &base(None),
            &competitor_result("allow"),
            &templates(None, None),
            &data,
            &mut Vec::new(),
        )
    });
    outcome.expect("an allowed intent passes");
    assert!(logs.contains("content filter competitor intent continues"));
    assert!(logs.contains("intent=competitor_comparison"));
    assert!(logs.contains("confidence=0.82"));
    assert!(logs.contains("action_hint=allow"));
    let (outcome, logs) = with_logs(|| {
        apply_competitor_intent_policy(
            &base(None),
            &competitor_result("refuse"),
            &templates(None, None),
            &data,
            &mut Vec::new(),
        )
    });
    outcome.expect_err("a refused intent fails");
    assert!(logs.contains("content filter competitor intent refuse"));
    let (outcome, logs) = with_logs(|| {
        apply_competitor_intent_policy(
            &base(None),
            &competitor_result("reframe"),
            &templates(None, None),
            &data,
            &mut Vec::new(),
        )
    });
    outcome.expect_err("a reframed intent fails the response");
    assert!(logs.contains("content filter competitor intent reframe"));
    assert!(logs.contains("action_hint=reframe"));
}
