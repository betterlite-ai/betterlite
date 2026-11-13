use std::time::Duration;

use serde::Serialize;

use super::support::{base, mock, mock_body, mock_slow};
use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::azure_content_safety::legacy_wire::AnalyzeTextResponse;

#[derive(Serialize)]
struct TextBody<'body> {
    text: &'body str,
}

#[tokio::test]
async fn posts_to_the_endpoint_with_the_azure_headers() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, None, None);
    let body = TextBody { text: "hello" };
    let response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    assert!(response.categories_analysis.is_empty());
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let request = seen.first().unwrap();
    assert_eq!(request.path, "/contentsafety/text:analyze");
    assert_eq!(request.query.as_deref(), Some("api-version=2024-09-01"));
    assert_eq!(request.subscription_key.as_deref(), Some("azure-key"));
    assert_eq!(request.content_type.as_deref(), Some("application/json"));
    assert_eq!(request.body, r#"{"text":"hello"}"#);
}

#[tokio::test]
async fn resolves_a_configured_api_version_in_the_url() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, Some("2023-01-01"), None);
    let body = TextBody { text: "hello" };
    let _response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    assert_eq!(
        seen.first().unwrap().query.as_deref(),
        Some("api-version=2023-01-01")
    );
}

#[tokio::test]
async fn resolves_the_javelin_version_to_the_modern_default() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, Some("v1"), None);
    let body = TextBody { text: "hello" };
    let _response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
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
    let guardrail_base = base(&server.base, None, None);
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
async fn passes_a_non_success_json_body_back_to_the_caller() {
    let server = mock(400, r#"{"categoriesAnalysis":[]}"#).await;
    let guardrail_base = base(&server.base, None, None);
    let body = TextBody { text: "hello" };
    let response: AnalyzeTextResponse = guardrail_base
        .post_to_content_safety("text:analyze", &body)
        .await
        .unwrap();
    assert!(response.categories_analysis.is_empty());
}

#[tokio::test]
async fn reports_a_response_that_is_not_json() {
    let server = mock(200, "not json").await;
    let guardrail_base = base(&server.base, None, None);
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
    let guardrail_base = base(&server.base, None, Some(Duration::from_millis(50)));
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Timeout { .. }));
}

#[tokio::test]
async fn rejects_a_response_larger_than_the_limit() {
    let server = mock_body(200, " ".repeat(1_048_577)).await;
    let guardrail_base = base(&server.base, None, None);
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::ResponseTooLarge { limit } if limit == 1_048_576
    ));
}

#[tokio::test]
async fn reports_a_failed_request() {
    let guardrail_base = base("http://127.0.0.1:1", None, None);
    let body = TextBody { text: "hello" };
    let error = guardrail_base
        .post_to_content_safety::<_, AnalyzeTextResponse>("text:analyze", &body)
        .await
        .unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Request { .. }));
}
