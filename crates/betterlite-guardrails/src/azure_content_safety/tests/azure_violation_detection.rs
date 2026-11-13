use reqwest::StatusCode;

use super::support::{legacy, mock, mock_body, threshold_map};
use crate::azure_content_safety::error::{AzureContentSafetyError, VIOLATION_MESSAGE};
use crate::azure_content_safety::legacy::LegacyAzureContentSafety;
use crate::azure_content_safety::legacy_wire::{AnalyzeTextResponse, TextCategory};

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
    let seen = server.seen.lock().unwrap();
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
    let server = mock_body(200, " ".repeat(1_048_577)).await;
    let hook = legacy(&server.base, None);
    let error = hook.test_violation("hello", None).await.unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::ResponseTooLarge { limit } if limit == 1_048_576
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
    let hate = result.get(&TextCategory::Hate).unwrap();
    assert!(!hate.filtered);
    assert_eq!(hate.severity, 5);
    let sexual = result.get(&TextCategory::Sexual).unwrap();
    assert!(sexual.filtered);
    assert_eq!(sexual.severity, 5);
    assert!(result.get(&TextCategory::Violence).is_none());
}

#[test]
fn keeps_the_last_severity_for_a_duplicated_category() {
    let hook = legacy("http://127.0.0.1:1", None);
    let response: AnalyzeTextResponse = serde_json::from_str(
        r#"{"categoriesAnalysis":[{"category":"Hate","severity":1},{"category":"Hate","severity":6}]}"#,
    )
    .unwrap();
    let result = hook.compute_result(&response);
    assert_eq!(result.get(&TextCategory::Hate).unwrap().severity, 6);
    assert!(result.get(&TextCategory::Hate).unwrap().filtered);
}
