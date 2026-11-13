use reqwest::StatusCode;

use super::support::{legacy, mock};
use crate::azure_content_safety::error::LegacyAzureContentSafetyError;
use crate::azure_content_safety::legacy_wire::{ModelResponse, TextCategory};

const VIOLATION: &str = r#"{"categoriesAnalysis":[{"category":"Violence","severity":6}]}"#;
const CLEAN: &str = r#"{"categoriesAnalysis":[{"category":"Violence","severity":1}]}"#;

fn response(json: &str) -> ModelResponse {
    serde_json::from_str(json).unwrap()
}

fn one_message(content: &str) -> ModelResponse {
    response(&format!(
        r#"{{"choices":[{{"message":{{"content":"{content}"}}}}]}}"#
    ))
}

#[tokio::test]
async fn blocks_violating_response_content() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let model_response = one_message("bad");
    let error = hook
        .async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        LegacyAzureContentSafetyError::Violation {
            category: TextCategory::Violence,
            severity: 6,
            ..
        }
    ));
    assert_eq!(error.detail().unwrap().source, Some("output"));
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn skips_a_content_list() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let model_response =
        response(r#"{"choices":[{"message":{"content":[{"type":"text","text":"bad"}]}}]}"#);
    hook.async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn skips_a_missing_message_or_content() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let model_response = response(r#"{"choices":[{},{"message":null},{"message":{}}]}"#);
    hook.async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn passes_when_the_response_is_not_a_model_response() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    hook.async_post_call_success_hook(None).await.unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn propagates_a_transport_failure_unmodified() {
    let server = mock(500, "{}").await;
    let hook = legacy(&server.base, None);
    let model_response = one_message("bad");
    let error = hook
        .async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        LegacyAzureContentSafetyError::HttpStatus { status } if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[tokio::test]
async fn scans_every_choice() {
    let server = mock(200, CLEAN).await;
    let hook = legacy(&server.base, None);
    let model_response =
        response(r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}}]}"#);
    hook.async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap();
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn stops_on_the_first_violation() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let model_response =
        response(r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}}]}"#);
    hook.async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap_err();
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn passes_clean_content() {
    let server = mock(200, CLEAN).await;
    let hook = legacy(&server.base, None);
    let model_response = one_message("fine");
    hook.async_post_call_success_hook(Some(&model_response))
        .await
        .unwrap();
}
