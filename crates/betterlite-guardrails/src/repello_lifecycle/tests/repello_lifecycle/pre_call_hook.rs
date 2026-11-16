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
    BLOCKED_REPLY, CapturedLog, GUARDRAIL_NAME, PASSED_REPLY, applied, data, guardrail_with,
    mock_repello, params, prompt_data, records, request_count, run_hook, settings, text_field,
};

fn guarding(base: &str, mode: &str) -> RepelloAILifecycle {
    let mut configured = params();
    configured.mode = LitellmParamsMode::One(mode.to_owned());
    guardrail_with(&configured, settings(base))
}

#[tokio::test]
async fn the_configured_event_scans_the_prompt_and_records_the_guardrail() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_hook(
        &guarding(&mock.base, "pre_call"),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(request_count(&mock), 1);
    assert_eq!(
        applied(&run.data),
        vec![GuardrailValue::Text(String::from(GUARDRAIL_NAME))]
    );
    assert_eq!(records(&run.data).len(), 1);
    assert_eq!(
        text_field(&records(&run.data)[0], "guardrail_mode").as_deref(),
        Some("pre_call")
    );
}

#[rstest]
#[case::post_call(LitellmParamsMode::One(String::from("post_call")))]
#[case::post_call_list(LitellmParamsMode::Many(vec![String::from("post_call")]))]
#[tokio::test]
async fn another_event_hook_skips_the_scan(#[case] mode: LitellmParamsMode) {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let mut configured = params();
    configured.mode = mode;
    let run = run_hook(
        &guardrail_with(&configured, settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(request_count(&mock), 0);
    assert!(records(&run.data).is_empty());
    assert!(applied(&run.data).is_empty());
}

#[tokio::test]
async fn a_suppressed_guardrail_skips_the_scan() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let mut suppressed = BTreeSet::new();
    suppressed.insert(String::from(GUARDRAIL_NAME));
    let run = run_hook(
        &guarding(&mock.base, "pre_call"),
        prompt_data(),
        HookTarget::Prompt,
        &suppressed,
    )
    .await;
    run.result.unwrap();
    assert_eq!(request_count(&mock), 0);
    assert!(records(&run.data).is_empty());
}

#[rstest]
#[case::no_text(r#"{"model": "gpt-4o"}"#)]
#[case::empty_content(r#"{"messages": [{"role": "user", "content": ""}]}"#)]
#[case::no_messages(r#"{"input": []}"#)]
#[tokio::test]
async fn a_request_without_prompt_text_warns_and_skips(#[case] payload: &str) {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let guardrail = guarding(&mock.base, "pre_call");
    let run = run_hook(
        &guardrail,
        data(payload),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(request_count(&mock), 0);
    assert!(records(&run.data).is_empty());
    assert!(
        captured
            .text()
            .contains("RepelloAI Argus: no inspectable prompt text in data - skipping."),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn a_blocked_prompt_is_a_400_before_the_guardrail_is_recorded_as_applied() {
    let mock = mock_repello(StatusCode::OK, BLOCKED_REPLY).await;
    let run = run_hook(
        &guarding(&mock.base, "pre_call"),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::Http { status, detail }) = run.result else {
        panic!("a blocked verdict must fail the request");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail,
        "Blocked by RepelloAI Argus guardrail. Policies violated: pii (action: block)."
    );
    assert!(applied(&run.data).is_empty());
    assert_eq!(
        text_field(&records(&run.data)[0], "guardrail_status").as_deref(),
        Some("guardrail_intervened")
    );
}
