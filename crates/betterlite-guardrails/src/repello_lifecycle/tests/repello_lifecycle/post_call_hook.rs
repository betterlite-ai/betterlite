use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::RepelloAILifecycle;
use betterlite_guardrails::repello_lifecycle::hooks::HookTarget;
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::support::{
    BLOCKED_REPLY, CapturedLog, FLAGGED_REPLY, PASSED_REPLY, applied, captured_path, data,
    guardrail_with, mock_repello, params, prompt_data, records, request_count, response_data,
    run_hook, settings, text_field,
};

fn guarding(base: &str, mode: &str) -> RepelloAILifecycle {
    let mut configured = params();
    configured.mode = LitellmParamsMode::One(mode.to_owned());
    guardrail_with(&configured, settings(base))
}

#[tokio::test]
async fn the_response_text_is_scanned_and_the_guardrail_is_recorded() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_hook(
        &guarding(&mock.base, "post_call"),
        prompt_data(),
        HookTarget::Response(&response_data()),
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(captured_path(&mock), "/analyze/response");
    assert_eq!(
        applied(&run.data),
        vec![GuardrailValue::Text(String::from("repello-test"))]
    );
    assert_eq!(
        text_field(&records(&run.data)[0], "guardrail_status").as_deref(),
        Some("success")
    );
    assert_eq!(
        text_field(&records(&run.data)[0], "guardrail_mode").as_deref(),
        Some("post_call")
    );
}

#[rstest]
#[case::empty_choices(r#"{"choices": []}"#)]
#[case::empty_content(
    r#"{"choices": [{"index": 0, "message": {"role": "assistant", "content": ""}}]}"#
)]
#[case::no_choices(r#"{"model": "gpt-4o"}"#)]
#[tokio::test]
async fn a_response_without_text_warns_and_skips(#[case] payload: &str) {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let guardrail = guarding(&mock.base, "post_call");
    let run = run_hook(
        &guardrail,
        prompt_data(),
        HookTarget::Response(&data(payload)),
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(request_count(&mock), 0);
    assert!(records(&run.data).is_empty());
    assert!(
        captured
            .text()
            .contains("RepelloAI Argus: no inspectable response text - skipping."),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn a_blocked_response_is_a_400() {
    let mock = mock_repello(StatusCode::OK, BLOCKED_REPLY).await;
    let run = run_hook(
        &guarding(&mock.base, "post_call"),
        prompt_data(),
        HookTarget::Response(&response_data()),
        &BTreeSet::new(),
    )
    .await;
    let Err(error) = run.result else {
        panic!("a blocked verdict must fail the response");
    };
    assert!(matches!(error, GuardrailFailure::Http { status: 400, .. }));
    assert!(applied(&run.data).is_empty());
}

#[tokio::test]
async fn a_flagged_response_is_allowed_and_logged() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, FLAGGED_REPLY).await;
    let run = run_hook(
        &guarding(&mock.base, "post_call"),
        prompt_data(),
        HookTarget::Response(&response_data()),
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert!(
        captured
            .text()
            .contains("RepelloAI Argus flagged content (allowed):"),
        "{}",
        captured.text()
    );
    assert_eq!(applied(&run.data).len(), 1);
}

#[tokio::test]
async fn a_pre_call_only_mode_skips_the_response_scan() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_hook(
        &guarding(&mock.base, "pre_call"),
        prompt_data(),
        HookTarget::Response(&response_data()),
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(request_count(&mock), 0);
    assert!(records(&run.data).is_empty());
}
