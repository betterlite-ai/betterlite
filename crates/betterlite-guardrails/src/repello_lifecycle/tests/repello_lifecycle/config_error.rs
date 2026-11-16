use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::hooks::HookTarget;
use betterlite_guardrails::repello_lifecycle::{
    MISCONFIGURED_ERROR_TYPE, MISCONFIGURED_MESSAGE, RepelloAILifecycle,
};
use rstest::rstest;

use crate::support::{
    field, guardrail, mock_repello, prompt_data, records, run_hook, settings, text_field,
};

fn guarding(base: &str, fallback: UnreachableFallback) -> RepelloAILifecycle {
    let mut configured = settings(base);
    configured.unreachable_fallback = fallback;
    guardrail(configured)
}

#[rstest]
#[case::bad_request(StatusCode::BAD_REQUEST, 400)]
#[case::unauthorized(StatusCode::UNAUTHORIZED, 401)]
#[case::forbidden(StatusCode::FORBIDDEN, 403)]
#[case::not_found(StatusCode::NOT_FOUND, 404)]
#[case::unprocessable(StatusCode::UNPROCESSABLE_ENTITY, 422)]
#[tokio::test]
async fn every_configuration_status_is_a_permanent_500_even_under_fail_open(
    #[case] answered: StatusCode,
    #[case] reported: u64,
) {
    let mock = mock_repello(answered, r#"{"error": "bad key"}"#).await;
    let run = run_hook(
        &guarding(&mock.base, UnreachableFallback::FailOpen),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = run.result else {
        panic!("expected a misconfiguration failure");
    };
    assert_eq!(status, 500);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(MISCONFIGURED_MESSAGE)))
    );
    assert_eq!(
        detail.get("status_code"),
        Some(&GuardrailValue::UnsignedInteger(reported))
    );
    assert_eq!(
        detail.get("error_type"),
        Some(&GuardrailValue::Text(String::from(
            MISCONFIGURED_ERROR_TYPE
        )))
    );
}

#[rstest]
#[case::fail_closed(UnreachableFallback::FailClosed)]
#[case::fail_open(UnreachableFallback::FailOpen)]
#[tokio::test]
async fn a_misconfigured_scan_records_the_failure_and_its_status(
    #[case] fallback: UnreachableFallback,
) {
    let mock = mock_repello(StatusCode::UNAUTHORIZED, r#"{"error": "bad key"}"#).await;
    let run = run_hook(
        &guarding(&mock.base, fallback),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    assert!(run.result.is_err());
    let recorded = records(&run.data);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        text_field(&recorded[0], "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
    let response = field(&recorded[0], "guardrail_response").unwrap();
    assert_eq!(
        text_field(response, "error").as_deref(),
        Some(MISCONFIGURED_MESSAGE)
    );
    // the record is rebuilt from its JSON, which reads a non-negative number back as signed
    assert_eq!(
        field(response, "status_code"),
        Some(&GuardrailValue::SignedInteger(401))
    );
}

#[rstest]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::teapot(StatusCode::IM_A_TEAPOT)]
#[case::too_many_requests(StatusCode::TOO_MANY_REQUESTS)]
#[tokio::test]
async fn a_status_outside_the_configuration_set_is_transient(#[case] status: StatusCode) {
    let mock = mock_repello(status, "{}").await;
    let run = run_hook(
        &guarding(&mock.base, UnreachableFallback::FailOpen),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    let recorded = records(&run.data);
    assert_eq!(
        text_field(&recorded[0], "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
}
