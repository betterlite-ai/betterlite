use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::hooks::HookTarget;
use betterlite_guardrails::repello_lifecycle::{
    RepelloAILifecycle, UNREACHABLE_ERROR_TYPE, UNREACHABLE_MESSAGE,
};
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::support::{
    CapturedLog, applied, closed_base, field, guardrail, mock_repello, prompt_data, records,
    run_hook, settings, text_field,
};

fn guarding(base: &str, fallback: UnreachableFallback) -> RepelloAILifecycle {
    let mut configured = settings(base);
    configured.unreachable_fallback = fallback;
    guardrail(configured)
}

#[tokio::test]
async fn fail_closed_turns_a_server_error_into_an_unreachable_500() {
    let mock = mock_repello(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let run = run_hook(
        &guarding(&mock.base, UnreachableFallback::FailClosed),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = run.result else {
        panic!("expected an unreachable failure");
    };
    assert_eq!(status, 500);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(UNREACHABLE_MESSAGE)))
    );
    assert_eq!(
        detail.get("error_type"),
        Some(&GuardrailValue::Text(String::from(UNREACHABLE_ERROR_TYPE)))
    );
    let recorded = records(&run.data);
    assert_eq!(
        text_field(&recorded[0], "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
    let Some(GuardrailValue::Text(reason)) = field(&recorded[0], "guardrail_response") else {
        panic!("expected the failure reason");
    };
    assert!(reason.contains("500"), "{reason}");
}

#[tokio::test]
async fn fail_open_allows_the_request_and_still_records_the_failure() {
    let mock = mock_repello(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let run = run_hook(
        &guarding(&mock.base, UnreachableFallback::FailOpen),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    assert!(run.result.is_ok());
    assert_eq!(
        applied(&run.data),
        vec![GuardrailValue::Text(String::from("repello-test"))]
    );
    let recorded = records(&run.data);
    assert_eq!(
        text_field(&recorded[0], "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
    let response = field(&recorded[0], "guardrail_response").unwrap();
    let GuardrailValue::Text(reason) = response else {
        panic!("expected the failure reason");
    };
    assert!(reason.contains("500"), "{reason}");
}

#[rstest]
#[case::fail_closed(UnreachableFallback::FailClosed)]
#[case::fail_open(UnreachableFallback::FailOpen)]
#[tokio::test]
async fn an_unreachable_api_warns_with_the_reason(#[case] fallback: UnreachableFallback) {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let run = run_hook(
        &guarding(&closed_base(), fallback),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let log = captured.text();
    assert!(log.contains("RepelloAI Argus unreachable: "), "{log}");
    assert_eq!(
        run.result.is_ok(),
        fallback == UnreachableFallback::FailOpen
    );
}

#[tokio::test]
async fn a_failed_scan_never_reports_a_misconfiguration() {
    let mock = mock_repello(StatusCode::BAD_GATEWAY, "{}").await;
    let run = run_hook(
        &guarding(&mock.base, UnreachableFallback::FailClosed),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { detail, .. }) = run.result else {
        panic!("expected an unreachable failure");
    };
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(UNREACHABLE_MESSAGE)))
    );
}
