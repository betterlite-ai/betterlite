use std::time::{Duration, Instant};

use jiff::Timestamp;
use reqwest::StatusCode;
use rstest::rstest;

use super::support::{Reply, legacy, mock, mock_body, mock_slow, mock_with, reply, threshold_map};
use crate::azure_content_safety::error::{AzureContentSafetyError, VIOLATION_MESSAGE};
use crate::azure_content_safety::legacy::LegacyAzureContentSafety;
use crate::azure_content_safety::legacy_wire::{AnalyzeTextResponse, TextCategory};

mod retry_policy;

const HATEFUL: &str = r#"{"categoriesAnalysis":[{"category":"Hate","severity":6}]}"#;

async fn hook_against(payload: &'static str) -> (super::support::Mock, LegacyAzureContentSafety) {
    let server = mock(200, payload).await;
    let hook = legacy(&server.base, None);
    (server, hook)
}

#[tokio::test]
async fn raises_when_the_severity_meets_the_threshold() {
    let (_server, hook) =
        hook_against(r#"{"categoriesAnalysis":[{"category":"Hate","severity":4}]}"#).await;
    let error = hook
        .test_violation("hello", Some("input"))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    let detail = error.detail().unwrap();
    assert_eq!(detail.error, VIOLATION_MESSAGE);
    assert_eq!(detail.source, Some("input"));
    assert_eq!(detail.category, TextCategory::Hate);
    assert_eq!(detail.severity, 4);
}

#[tokio::test]
async fn raises_when_the_severity_is_above_the_threshold() {
    let (_server, hook) =
        hook_against(r#"{"categoriesAnalysis":[{"category":"Sexual","severity":6}]}"#).await;
    let error = hook.test_violation("hello", None).await.unwrap_err();
    let detail = error.detail().unwrap();
    assert_eq!(detail.category, TextCategory::Sexual);
    assert_eq!(detail.severity, 6);
    assert_eq!(detail.source, None);
}

#[tokio::test]
async fn passes_when_every_severity_is_below_the_threshold() {
    let (_server, hook) = hook_against(
        r#"{"categoriesAnalysis":[{"category":"Hate","severity":3},{"category":"Sexual","severity":2}]}"#,
    )
    .await;
    hook.test_violation("hello", Some("input")).await.unwrap();
}

#[tokio::test]
async fn reports_the_first_filtered_category_in_category_order() {
    let (_server, hook) = hook_against(
        r#"{"categoriesAnalysis":[{"category":"Violence","severity":7},{"category":"Hate","severity":5}]}"#,
    )
    .await;
    let error = hook
        .test_violation("hello", Some("input"))
        .await
        .unwrap_err();
    assert_eq!(error.detail().unwrap().category, TextCategory::Hate);
}

#[tokio::test]
async fn skips_a_category_the_response_omits() {
    let (_server, hook) =
        hook_against(r#"{"categoriesAnalysis":[{"category":"Violence","severity":5}]}"#).await;
    let error = hook
        .test_violation("hello", Some("input"))
        .await
        .unwrap_err();
    assert_eq!(error.detail().unwrap().category, TextCategory::Violence);
}

#[tokio::test]
async fn skips_a_category_with_a_null_severity() {
    let (_server, hook) = hook_against(
        r#"{"categoriesAnalysis":[{"category":"Hate","severity":null},{"category":"Violence","severity":5}]}"#,
    )
    .await;
    let error = hook
        .test_violation("hello", Some("input"))
        .await
        .unwrap_err();
    assert_eq!(error.detail().unwrap().category, TextCategory::Violence);
}

#[tokio::test]
async fn sends_the_legacy_sdk_request() {
    let (server, hook) = hook_against(r#"{"categoriesAnalysis":[]}"#).await;
    hook.test_violation("hello", Some("input")).await.unwrap();
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let request = seen.first().unwrap();
    assert_eq!(request.path, "/contentsafety/text:analyze");
    assert_eq!(request.query.as_deref(), Some("api-version=2023-10-01"));
    assert_eq!(request.subscription_key.as_deref(), Some("azure-key"));
    assert_eq!(request.content_type.as_deref(), Some("application/json"));
    assert_eq!(
        request.body,
        r#"{"text":"hello","outputType":"EightSeverityLevels"}"#
    );
}

#[tokio::test]
async fn surfaces_an_http_error_status() {
    let server = mock(500, "{}").await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::HttpStatus { status, .. } if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[tokio::test]
async fn copies_a_throttling_status_for_the_post_call_hook() {
    let server = mock(429, "{}").await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn maps_a_status_outside_the_error_range_to_server_error() {
    let server = mock(302, "{}").await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn surfaces_a_transport_failure() {
    let hook = legacy("http://127.0.0.1:1", None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Request { .. }));
}

#[tokio::test]
async fn rejects_a_response_larger_than_the_limit() {
    let server = mock_body(200, &" ".repeat(0x0010_0001)).await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::ResponseTooLarge { limit } if limit == 0x0010_0000
    ));
}

#[test]
fn computes_a_verdict_for_every_reported_category() {
    let hook = legacy("http://127.0.0.1:1", Some(threshold_map(&[("Hate", 6)])));
    let response: AnalyzeTextResponse = serde_json::from_str(
        r#"{"categoriesAnalysis":[{"category":"Hate","severity":5},{"category":"Sexual","severity":5}]}"#,
    )
    .unwrap();
    let result = hook.compute_result(&response);
    assert_eq!(result.len(), 2);
    let hate = &result[&TextCategory::Hate];
    assert!(!hate.filtered);
    assert_eq!(hate.severity, 5);
    let sexual = &result[&TextCategory::Sexual];
    assert!(sexual.filtered);
    assert_eq!(sexual.severity, 5);
    assert!(!result.contains_key(&TextCategory::Violence));
}

#[test]
fn keeps_the_last_severity_for_a_duplicated_category() {
    let hook = legacy("http://127.0.0.1:1", None);
    let response: AnalyzeTextResponse = serde_json::from_str(
        r#"{"categoriesAnalysis":[{"category":"Hate","severity":1},{"category":"Hate","severity":6}]}"#,
    )
    .unwrap();
    let result = hook.compute_result(&response);
    assert_eq!(result[&TextCategory::Hate].severity, 6);
    assert!(result[&TextCategory::Hate].filtered);
}

#[rstest]
#[case::internal_server_error(500)]
#[case::service_unavailable(503)]
#[case::gateway_timeout(504)]
#[tokio::test]
async fn retries_a_server_error_and_scans_the_next_answer(#[case] status: u16) {
    let server = mock_with(vec![reply(status, "{}"), reply(200, HATEFUL)], None).await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::Violation {
            category: TextCategory::Hate,
            severity: 6,
            ..
        }
    ));
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[rstest]
#[case::bad_request(400)]
#[case::request_timeout(408)]
#[case::too_many_requests(429)]
#[case::bad_gateway(502)]
#[case::http_version_not_supported(505)]
#[case::variant_also_negotiates(506)]
#[tokio::test]
async fn does_not_retry_a_post_on_a_status_off_the_post_list(#[case] status: u16) {
    let server = mock_with(vec![reply(status, "{}"), reply(200, HATEFUL)], None).await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::HttpStatus { .. }));
    assert_eq!(error.status_code().as_u16(), status);
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn retries_any_error_status_that_sends_retry_after() {
    let throttled = Reply {
        status: 429,
        payload: "{}".to_owned(),
        headers: vec![("retry-after", "0")],
    };
    let server = mock_with(vec![throttled, reply(200, HATEFUL)], None).await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Violation { .. }));
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn gives_up_after_three_retries_with_the_last_status() {
    let server = mock(503, "{}").await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(server.seen.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn waits_the_provider_delay_before_sending_again() {
    let throttled = Reply {
        status: 503,
        payload: "{}".to_owned(),
        headers: vec![("retry-after-ms", "300")],
    };
    let server = mock_with(vec![throttled, reply(200, HATEFUL)], None).await;
    let hook = legacy(&server.base, None);
    let started = Instant::now();
    let error = hook.test_violation("hello", None).await.unwrap_err();
    let waited = started.elapsed();
    assert!(matches!(error, AzureContentSafetyError::Violation { .. }));
    assert_eq!(server.seen.lock().unwrap().len(), 2);
    assert!(
        waited >= Duration::from_millis(300),
        "the retry waited {waited:?} instead of the 300 ms the provider asked for"
    );
}

#[tokio::test]
async fn backs_off_between_retryable_server_errors() {
    let server = mock(503, "{}").await;
    let mut hook = legacy(&server.base, None);
    hook.retry_backoff_factor = Duration::from_millis(50);
    let started = Instant::now();
    let error = hook.test_violation("hello", None).await.unwrap_err();
    let waited = started.elapsed();
    assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(server.seen.lock().unwrap().len(), 4);
    assert!(
        waited >= Duration::from_millis(300),
        "the three retries waited {waited:?}, less than the 300 ms the backoff asks for"
    );
}

#[tokio::test]
async fn reads_the_injected_clock_for_a_retry_after_date() {
    let throttled = Reply {
        status: 503,
        payload: "{}".to_owned(),
        headers: vec![("retry-after", "Fri, 02 Jan 2026 03:04:06 GMT")],
    };
    let server = mock_with(vec![throttled, reply(200, HATEFUL)], None).await;
    let mut hook = legacy(&server.base, None);
    // Only a clock one second before the date turns it into a delay worth waiting.
    hook.clock = || Timestamp::from_second(1_767_323_045).unwrap();
    let started = Instant::now();
    let error = hook.test_violation("hello", None).await.unwrap_err();
    let waited = started.elapsed();
    assert!(matches!(error, AzureContentSafetyError::Violation { .. }));
    assert_eq!(server.seen.lock().unwrap().len(), 2);
    assert!(
        waited >= Duration::from_millis(900),
        "the retry waited {waited:?} instead of the second the date asked for"
    );
}

#[tokio::test]
async fn a_legacy_timeout_answers_with_a_server_error() {
    let server = mock_slow(200, "{}", Duration::from_millis(300)).await;
    let mut hook = legacy(&server.base, None);
    hook.timeout = Duration::from_millis(50);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Timeout { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}
