use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::vigil_guard::vigil_guard_analysis::{
    Analysis, Decision, Source, analyze,
};
use betterlite_guardrails::vigil_guard::vigil_guard_error::VigilGuardError;
use betterlite_guardrails::vigil_guard::vigil_guard_metadata::MetadataValue;
use indexmap::IndexMap;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::support::{FakeHandler, analysis, json_body, metadata, settings, text};

fn decision(analysis: &Analysis) -> Option<Decision> {
    analysis.decision.as_ref().and_then(Decision::from_value)
}

fn model_metadata() -> IndexMap<String, MetadataValue> {
    let mut values = metadata();
    values.insert("model".to_owned(), MetadataValue::Text("gpt-4o".to_owned()));
    values
}

#[tokio::test]
async fn sends_full_mode_payload_to_guard_analyze() {
    let handler = FakeHandler::new(vec![Ok(json_body(
        r#"{"decision":"BLOCKED","blockMessage":"banned content","sanitizedText":"clean"}"#,
    ))]);
    let metadata = model_metadata();
    let result = analyze(&handler, &settings(), "hello", Source::UserInput, &metadata)
        .await
        .expect("analysis succeeds");
    let calls = handler.calls();
    let call = &calls[0];
    assert_eq!(call.url, "https://vigil.example/v1/guard/analyze");
    assert_eq!(
        call.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer sk-vigil-test"
    );
    assert_eq!(
        call.headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
    assert_eq!(
        serde_json::to_string(&call.payload).unwrap(),
        r#"{"text":"hello","source":"user_input","mode":"full","metadata":{"model":"gpt-4o"}}"#
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(decision(&result), Some(Decision::Blocked));
    assert_eq!(result.block_message, Some(text("banned content")));
    assert_eq!(result.sanitized_text, Some(text("clean")));
}

#[tokio::test]
async fn model_output_source_is_sent_for_responses() {
    let handler = FakeHandler::new(vec![Ok(json_body(r#"{"decision":"ALLOWED"}"#))]);
    let result = analyze(
        &handler,
        &settings(),
        "reply",
        Source::ModelOutput,
        &metadata(),
    )
    .await
    .expect("analysis succeeds");
    let calls = handler.calls();
    assert_eq!(
        serde_json::to_string(&calls[0].payload).unwrap(),
        r#"{"text":"reply","source":"model_output","mode":"full","metadata":{}}"#
    );
    assert_eq!(decision(&result), Some(Decision::Allowed));
}

#[tokio::test]
async fn invalid_json_body_is_a_response_error() {
    let handler = FakeHandler::new(vec![Ok(json_body("not json"))]);
    let error = analyze(
        &handler,
        &settings(),
        "hello",
        Source::UserInput,
        &metadata(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, VigilGuardError::ResponseJson { .. }));
}

#[tokio::test]
async fn analysis_fields_load_from_camel_case_keys() {
    let handler = FakeHandler::new(vec![Ok(json_body(
        r#"{"decision":"SANITIZED","decisionReason":"pii","categories":["a","b"],"outputText":"masked"}"#,
    ))]);
    let result = analyze(
        &handler,
        &settings(),
        "hello",
        Source::UserInput,
        &metadata(),
    )
    .await
    .expect("analysis succeeds");
    assert_eq!(decision(&result), Some(Decision::Sanitized));
    assert_eq!(result.decision_reason, Some(text("pii")));
    assert_eq!(result.output_text, Some(text("masked")));
    assert_eq!(
        result.categories,
        Some(GuardrailValue::Array(vec![text("a"), text("b")]))
    );
}

#[tokio::test]
async fn unknown_decision_is_not_valid() {
    let result = analysis(r#"{"decision":"UNKNOWN"}"#);
    assert_eq!(decision(&result), None);
}
