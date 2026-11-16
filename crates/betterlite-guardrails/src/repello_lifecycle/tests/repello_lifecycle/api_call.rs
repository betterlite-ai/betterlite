use std::collections::BTreeSet;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::hooks::HookTarget;
use betterlite_guardrails::repello_lifecycle::{
    API_KEY_HEADER, Data, FAILED_MESSAGE, INVALID_JSON_ERROR_TYPE, INVALID_JSON_MESSAGE,
    PROMPT_STAGE, RESPONSE_STAGE, UNREACHABLE_ERROR_TYPE, UNREACHABLE_MESSAGE,
};
use rstest::rstest;
use secrecy::SecretString;
use tracing::subscriber::set_default;

use crate::support::{
    API_KEY, CapturedLog, PASSED_REPLY, captured_body, captured_headers, captured_path, field,
    guardrail, guardrail_with, mock_repello, mock_repello_after, params, prompt_data, records,
    request_count, response_data, run_hook, settings, text_field,
};

fn scan_text(body: &Data, stage: &str) -> Option<String> {
    text_field(body.get("scan_data")?, stage)
}

#[tokio::test]
async fn a_prompt_scan_posts_the_asset_and_the_prompt_text() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(captured_path(&mock), "/analyze/prompt");
    let headers = captured_headers(&mock);
    assert_eq!(
        headers.get(API_KEY_HEADER).unwrap().to_str().unwrap(),
        API_KEY
    );
    assert!(
        headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let body = captured_body(&mock);
    assert_eq!(
        body.get("asset_id"),
        Some(&GuardrailValue::Text(String::from("asset-123")))
    );
    assert_eq!(scan_text(&body, "prompt").as_deref(), Some("hello"));
    assert_eq!(scan_text(&body, "response"), None);
}

#[tokio::test]
async fn a_response_scan_posts_the_response_text() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let mut configured = params();
    configured.mode = LitellmParamsMode::One(String::from("post_call"));
    let run = run_hook(
        &guardrail_with(&configured, settings(&mock.base)),
        prompt_data(),
        HookTarget::Response(&response_data()),
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    assert_eq!(captured_path(&mock), "/analyze/response");
    let body = captured_body(&mock);
    assert_eq!(scan_text(&body, "response").as_deref(), Some("answer"));
    assert_eq!(scan_text(&body, "prompt"), None);
}

#[tokio::test]
async fn a_passed_scan_allows_the_request_and_records_it() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    let recorded = records(&run.data);
    assert_eq!(recorded.len(), 1);
    let info = &recorded[0];
    assert_eq!(
        text_field(info, "guardrail_name").as_deref(),
        Some("repello-test")
    );
    assert_eq!(
        text_field(info, "guardrail_status").as_deref(),
        Some("success")
    );
    let response = field(info, "guardrail_response").unwrap();
    assert_eq!(text_field(response, "verdict").as_deref(), Some("passed"));
}

#[tokio::test]
async fn the_record_carries_the_scan_timestamps() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let recorded = records(&run.data);
    let scan = &recorded[0];
    assert_eq!(
        field(scan, "start_time"),
        Some(&GuardrailValue::Decimal(1_700_000_000.0))
    );
    assert_eq!(
        field(scan, "end_time"),
        Some(&GuardrailValue::Decimal(1_700_000_000.0))
    );
    assert_eq!(field(scan, "duration"), Some(&GuardrailValue::Decimal(0.0)));
}

#[rstest]
#[case::a_json_string("\"nope\"")]
#[case::not_json("<html>")]
#[case::a_wrong_verdict_type("{\"verdict\": 5}")]
#[case::a_wrong_policy_type("{\"verdict\": \"blocked\", \"policies_violated\": 5}")]
#[tokio::test]
async fn an_answer_that_is_not_an_analyze_response_is_a_500(#[case] reply: &str) {
    let mock = mock_repello(StatusCode::OK, reply).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = run.result else {
        panic!("expected an invalid JSON failure");
    };
    assert_eq!(status, 500);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(INVALID_JSON_MESSAGE)))
    );
    assert_eq!(
        detail.get("error_type"),
        Some(&GuardrailValue::Text(String::from(INVALID_JSON_ERROR_TYPE)))
    );
    assert_eq!(
        detail.get("status_code"),
        Some(&GuardrailValue::UnsignedInteger(200))
    );
}

#[tokio::test]
async fn a_request_timeout_is_an_unreachable_api() {
    let mock = mock_repello_after(Duration::from_secs(5), StatusCode::OK, PASSED_REPLY).await;
    let mut configured = settings(&mock.base);
    configured.request_timeout = Duration::from_millis(50);
    let run = run_hook(
        &guardrail(configured),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = run.result else {
        panic!("expected a timeout failure");
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
}

#[tokio::test]
async fn a_timed_out_scan_passes_under_fail_open() {
    let mock = mock_repello_after(Duration::from_secs(5), StatusCode::OK, PASSED_REPLY).await;
    let mut configured = settings(&mock.base);
    configured.request_timeout = Duration::from_millis(50);
    configured.unreachable_fallback = UnreachableFallback::FailOpen;
    let run = run_hook(
        &guardrail(configured),
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

#[tokio::test]
async fn a_key_that_cannot_be_a_header_value_is_the_generic_failure_even_under_fail_open() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let mut configured = settings(&mock.base);
    configured.api_key = SecretString::from(String::from("key\nvalue"));
    configured.unreachable_fallback = UnreachableFallback::FailOpen;
    let run = run_hook(
        &guardrail(configured),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = run.result else {
        panic!("expected the generic failure");
    };
    assert_eq!(status, 500);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(FAILED_MESSAGE)))
    );
    assert_eq!(detail.get("error_type"), None);
    assert_eq!(request_count(&mock), 0);
    let recorded = records(&run.data);
    assert_eq!(
        text_field(&recorded[0], "guardrail_status").as_deref(),
        Some("guardrail_failed_to_respond")
    );
}

#[tokio::test]
async fn a_float_score_in_a_blocked_answer_is_printed_as_a_number() {
    let mock = mock_repello(
        StatusCode::OK,
        r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "details": {"score": 0.9}}]}"#,
    )
    .await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
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
        "Blocked by RepelloAI Argus guardrail. Policies violated: pii (score: 0.9)."
    );
}

#[tokio::test]
async fn a_float_nested_in_policies_applied_is_recorded_as_a_number() {
    let mock = mock_repello(
        StatusCode::OK,
        r#"{"verdict": "passed", "policies_applied": [{"meta": {"scores": [0.25], "note": "kept"}}]}"#,
    )
    .await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
    let recorded = records(&run.data);
    let response = field(&recorded[0], "guardrail_response").unwrap();
    let Some(GuardrailValue::Array(applied)) = field(response, "policies_applied") else {
        panic!("expected the applied policies");
    };
    let Some(GuardrailValue::Object(meta)) = field(&applied[0], "meta") else {
        panic!("expected the policy metadata");
    };
    assert_eq!(
        meta.get("note"),
        Some(&GuardrailValue::Text(String::from("kept")))
    );
    let Some(GuardrailValue::Array(scores)) = meta.get("scores") else {
        panic!("expected the scores");
    };
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0], GuardrailValue::Decimal(0.25));
}

#[tokio::test]
async fn an_answer_with_a_large_policy_list_is_read_whole() {
    let filler = "x".repeat(3_000);
    let reply = format!(r#"{{"verdict": "passed", "request_id": "{filler}"}}"#);
    let mock = mock_repello(StatusCode::OK, &reply).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    run.result.unwrap();
}

#[tokio::test]
async fn an_answer_over_the_response_cap_is_the_generic_failure() {
    let filler = "x".repeat(1024 * 1024);
    let reply = format!(r#"{{"verdict": "passed", "request_id": "{filler}"}}"#);
    let mock = mock_repello(StatusCode::OK, &reply).await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let Err(GuardrailFailure::HttpObject { status, detail }) = run.result else {
        panic!("expected the generic failure");
    };
    assert_eq!(status, 500);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(String::from(FAILED_MESSAGE)))
    );
    assert_eq!(detail.get("error_type"), None);
}

#[tokio::test]
async fn an_empty_reply_is_invalid_json() {
    let mock = mock_repello(StatusCode::OK, "").await;
    let run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    assert!(matches!(
        run.result,
        Err(GuardrailFailure::HttpObject { status: 500, .. })
    ));
    assert_eq!(request_count(&mock), 1);
}

#[tokio::test]
async fn the_scanned_text_never_reaches_the_log() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::TRACE));
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let _run = run_hook(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        HookTarget::Prompt,
        &BTreeSet::new(),
    )
    .await;
    let log = captured.text();
    assert!(!log.contains("hello"), "{log}");
    assert!(!log.contains("passed"), "{log}");
}

#[test]
fn the_scan_stage_names_are_the_litellm_path_segments() {
    assert_eq!(PROMPT_STAGE, "prompt");
    assert_eq!(RESPONSE_STAGE, "response");
}
