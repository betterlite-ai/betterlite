use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::{
    API_KEY_HEADER, FAILED_MESSAGE, INVALID_JSON_ERROR_TYPE, INVALID_JSON_MESSAGE, PROMPT_STAGE,
    RESPONSE_STAGE, UNREACHABLE_ERROR_TYPE, UNREACHABLE_MESSAGE,
};
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::support::{
    API_KEY, CapturedLog, Data, PASSED_REPLY, captured_body, captured_headers, captured_path,
    field, guardrail, mock_repello, mock_repello_after, prompt_data, records, request_count,
    response_data, run_post_call, run_pre_call, settings, text_field,
};

fn scan_text(body: &Data, stage: &str) -> Option<String> {
    text_field(body.get("scan_data")?, stage)
}

#[tokio::test]
async fn a_prompt_scan_posts_the_asset_and_the_prompt_text() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
    assert!(run.result.is_ok());
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
    let run = run_post_call(
        &guardrail(settings(&mock.base)),
        prompt_data(),
        &response_data(),
    )
    .await;
    assert!(run.result.is_ok());
    assert_eq!(captured_path(&mock), "/analyze/response");
    let body = captured_body(&mock);
    assert_eq!(scan_text(&body, "response").as_deref(), Some("answer"));
    assert_eq!(scan_text(&body, "prompt"), None);
}

#[tokio::test]
async fn a_passed_scan_allows_the_request_and_records_it() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
    assert!(run.result.is_ok());
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
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
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
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
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
    let run = run_pre_call(&guardrail(configured), prompt_data()).await;
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
async fn an_answer_with_a_large_policy_list_is_read_whole() {
    let filler = "x".repeat(3_000);
    let reply = format!(r#"{{"verdict": "passed", "request_id": "{filler}"}}"#);
    let mock = mock_repello(StatusCode::OK, &reply).await;
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
    assert!(run.result.is_ok());
}

#[tokio::test]
async fn an_answer_over_the_response_cap_is_the_generic_failure() {
    let filler = "x".repeat(1024 * 1024);
    let reply = format!(r#"{{"verdict": "passed", "request_id": "{filler}"}}"#);
    let mock = mock_repello(StatusCode::OK, &reply).await;
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
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
    let run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
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
    let _run = run_pre_call(&guardrail(settings(&mock.base)), prompt_data()).await;
    let log = captured.text();
    assert!(!log.contains("hello"), "{log}");
    assert!(!log.contains("passed"), "{log}");
}

#[test]
fn the_scan_stage_names_are_the_litellm_path_segments() {
    assert_eq!(PROMPT_STAGE, "prompt");
    assert_eq!(RESPONSE_STAGE, "response");
}
