#![cfg(test)]

use std::time::Duration;

use indexmap::IndexMap;

use super::shared::{anonymizer, item, json_reply, request_data, spawn_anonymizer, tokens};
use crate::presidio_actions::anonymize::{AnonymizeTextCall, anonymize_text};
use crate::presidio_actions::error::PresidioActionsError;

#[tokio::test]
async fn empty_analysis_results_return_the_original_text_without_a_request() {
    let mock = spawn_anonymizer(|_| json_reply(r#"{"text":"MASKED"}"#), Duration::ZERO).await;
    let mut counts = IndexMap::new();
    let text = anonymize_text(
        &anonymizer(&mock.base),
        AnonymizeTextCall {
            text: "Alice",
            analyze_results: &[],
            output_parse_pii: false,
            masked_entity_count: &mut counts,
            request_data: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(text, "Alice");
    assert!(mock.requests.lock().unwrap().is_empty());
    assert!(counts.is_empty());
}

#[tokio::test]
async fn simple_mode_returns_the_anonymizer_text_and_counts_its_items() {
    let mock = spawn_anonymizer(
        |_| json_reply(r#"{"text":"<PERSON>","items":[{"entity_type":"PERSON"}]}"#),
        Duration::ZERO,
    )
    .await;
    let mut counts = IndexMap::new();
    let text = anonymize_text(
        &anonymizer(&mock.base),
        AnonymizeTextCall {
            text: "Alice",
            analyze_results: &[item("PERSON", 0, 5)],
            output_parse_pii: false,
            masked_entity_count: &mut counts,
            request_data: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(text, "<PERSON>");
    assert_eq!(counts.get("PERSON"), Some(&1));
}

#[tokio::test]
async fn numbered_mode_rebuilds_the_text_and_stores_tokens() {
    let mock = spawn_anonymizer(|_| json_reply(r#"{"text":"IGNORED"}"#), Duration::ZERO).await;
    let mut data = request_data("{}");
    let mut counts = IndexMap::new();
    let text = anonymize_text(
        &anonymizer(&mock.base),
        AnonymizeTextCall {
            text: "Hello Alice",
            analyze_results: &[item("PERSON", 6, 11)],
            output_parse_pii: true,
            masked_entity_count: &mut counts,
            request_data: Some(&mut data),
        },
    )
    .await
    .unwrap();
    assert_eq!(text, "Hello <PERSON_1>");
    assert_eq!(
        tokens(&data).get("<PERSON_1>").map(String::as_str),
        Some("Alice")
    );
    assert_eq!(counts.get("PERSON"), Some(&1));
}

#[tokio::test]
async fn numbered_mode_without_request_data_skips_token_storage() {
    let mock = spawn_anonymizer(|_| json_reply(r#"{"text":"IGNORED"}"#), Duration::ZERO).await;
    let mut counts = IndexMap::new();
    let text = anonymize_text(
        &anonymizer(&mock.base),
        AnonymizeTextCall {
            text: "Hello Alice",
            analyze_results: &[item("PERSON", 6, 11)],
            output_parse_pii: true,
            masked_entity_count: &mut counts,
            request_data: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(text, "Hello <PERSON_1>");
}

#[tokio::test]
async fn an_unparsable_anonymizer_response_is_sanitized() {
    let mock = spawn_anonymizer(|_| json_reply("not json"), Duration::ZERO).await;
    let mut counts = IndexMap::new();
    let error = anonymize_text(
        &anonymizer(&mock.base),
        AnonymizeTextCall {
            text: "SECRET Alice",
            analyze_results: &[item("PERSON", 7, 12)],
            output_parse_pii: false,
            masked_entity_count: &mut counts,
            request_data: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        PresidioActionsError::AnonymizerResponseJson { .. }
    ));
    assert!(!error.to_string().contains("SECRET"));
}
