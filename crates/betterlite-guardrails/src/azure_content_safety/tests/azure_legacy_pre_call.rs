use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use reqwest::StatusCode;

use super::support::{legacy, mock, mock_with, reply};
use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::azure_content_safety::legacy_hooks::async_pre_call_hook;
use crate::azure_content_safety::legacy_wire::TextCategory;

const VIOLATION: &str = r#"{"categoriesAnalysis":[{"category":"Hate","severity":5}]}"#;
const HATEFUL: &str = r#"{"categoriesAnalysis":[{"category":"Hate","severity":6}]}"#;
const CLEAN: &str = r#"{"categoriesAnalysis":[{"category":"Hate","severity":1}]}"#;

fn data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

#[tokio::test]
async fn does_not_scan_a_call_type_without_free_text() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let request = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);
    async_pre_call_hook(&hook, &request, "embedding")
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn does_not_scan_the_sync_responses_call_type() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let request = data(r#"{"input":"bad"}"#);
    async_pre_call_hook(&hook, &request, "responses")
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn blocks_a_violating_chat_message() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let request = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);
    let error = async_pre_call_hook(&hook, &request, "completion")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::Violation {
            category: TextCategory::Hate,
            severity: 5,
            ..
        }
    ));
    assert_eq!(error.detail().unwrap().source, Some("input"));
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn scans_a_responses_input() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    let request = data(r#"{"input":"bad"}"#);
    let error = async_pre_call_hook(&hook, &request, "aresponses")
        .await
        .unwrap_err();
    assert!(matches!(error, AzureContentSafetyError::Violation { .. }));
}

#[tokio::test]
async fn passes_clean_messages_and_scans_every_text() {
    let server = mock(200, CLEAN).await;
    let hook = legacy(&server.base, None);
    let request =
        data(r#"{"messages":[{"role":"user","content":"one"},{"role":"user","content":"two"}]}"#);
    async_pre_call_hook(&hook, &request, "acompletion")
        .await
        .unwrap();
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn suppresses_a_transport_failure_and_stops_scanning() {
    let server = mock(500, "{}").await;
    let hook = legacy(&server.base, None);
    let request =
        data(r#"{"messages":[{"role":"user","content":"one"},{"role":"user","content":"two"}]}"#);
    async_pre_call_hook(&hook, &request, "completion")
        .await
        .unwrap();
    // The SDK sends the first text four times before it gives up, and the second is never sent.
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 4);
    assert!(
        seen.iter()
            .all(|sent| sent.body.contains(r#""text":"one""#))
    );
}

#[tokio::test]
async fn blocks_a_violation_found_after_a_retried_outage() {
    let server = mock_with(vec![reply(503, "{}"), reply(200, HATEFUL)], None).await;
    let hook = legacy(&server.base, None);
    let request = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);
    let error = async_pre_call_hook(&hook, &request, "completion")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::Violation {
            category: TextCategory::Hate,
            severity: 6,
            ..
        }
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn passes_when_the_request_carries_no_text() {
    let server = mock(200, VIOLATION).await;
    let hook = legacy(&server.base, None);
    async_pre_call_hook(&hook, &data("{}"), "completion")
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}
