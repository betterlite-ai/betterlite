use std::time::Duration;

use reqwest::StatusCode;
use serde::Serialize;

use super::support::{base, mock, mock_body, mock_slow};
use crate::azure_content_safety::api_version::DEFAULT_API_VERSION;
use crate::azure_content_safety::config::AzureContentSafetyConfigModel;
use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::azure_content_safety::legacy_wire::AnalyzeTextResponse;

#[derive(Serialize)]
struct TextBody<'body> {
    text: &'body str,
}

#[tokio::test]
async fn posts_to_the_endpoint_with_the_azure_headers() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, DEFAULT_API_VERSION, None);
    let body = TextBody { text: "hello" };
    let response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    assert!(response.categories_analysis.is_empty());
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let request = seen.first().unwrap();
    assert_eq!(request.path, "/contentsafety/text:analyze");
    assert_eq!(request.query.as_deref(), Some("api-version=2024-09-01"));
    assert_eq!(request.subscription_key.as_deref(), Some("azure-key"));
    assert_eq!(request.content_type.as_deref(), Some("application/json"));
    assert_eq!(request.body, r#"{"text":"hello"}"#);
}

#[tokio::test]
async fn sends_a_configured_api_version_in_the_url() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, "2023-01-01", None);
    let body = TextBody { text: "hello" };
    let _response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(
        seen.first().unwrap().query.as_deref(),
        Some("api-version=2023-01-01")
    );
}

#[tokio::test]
async fn resolves_the_javelin_version_to_the_modern_default_before_the_post() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let model: AzureContentSafetyConfigModel =
        serde_json::from_str(r#"{"api_version":"v1"}"#).unwrap();
    let guardrail_base = base(&server.base, &model.api_version, None);
    let body = TextBody { text: "hello" };
    let _response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(
        seen.first().unwrap().query.as_deref(),
        Some("api-version=2024-09-01")
    );
}

#[tokio::test]
async fn returns_the_parsed_response_json() {
    let server = mock(
        200,
        r#"{"categoriesAnalysis":[{"category":"Hate","severity":4}]}"#,
    )
    .await;
    let guardrail_base = base(&server.base, DEFAULT_API_VERSION, None);
    let body = TextBody { text: "hello" };
    let response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    assert_eq!(response.categories_analysis.len(), 1);
    assert_eq!(
        response.categories_analysis.first().unwrap().severity,
        Some(4)
    );
}

#[tokio::test]
async fn reports_a_non_success_status_with_its_bounded_body() {
    let server = mock(401, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, DEFAULT_API_VERSION, None);
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::HttpStatus { status, .. } if status == StatusCode::UNAUTHORIZED
    ));
    let AzureContentSafetyError::HttpStatus {
        body: error_body, ..
    } = error
    else {
        panic!("expected an HTTP status error");
    };
    assert_eq!(error_body, r#"{"categoriesAnalysis":[]}"#);
}

#[tokio::test]
async fn reports_a_response_that_is_not_json() {
    let server = mock(200, "not json").await;
    let guardrail_base = base(&server.base, DEFAULT_API_VERSION, None);
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::ResponseJson { .. }
    ));
}

#[tokio::test]
async fn times_out_after_the_configured_timeout() {
    let server = mock_slow(200, "{}", Duration::from_millis(300)).await;
    let guardrail_base = base(
        &server.base,
        DEFAULT_API_VERSION,
        Some(Duration::from_millis(50)),
    );
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Timeout { .. }));
    assert_eq!(error.status_code(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn rejects_a_response_larger_than_the_limit() {
    let server = mock_body(200, &" ".repeat(0x0010_0001)).await;
    let guardrail_base = base(&server.base, DEFAULT_API_VERSION, None);
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::ResponseTooLarge { limit } if limit == 0x0010_0000
    ));
}

#[tokio::test]
async fn reports_a_failed_request() {
    let guardrail_base = base("http://127.0.0.1:1", DEFAULT_API_VERSION, None);
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Request { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}
