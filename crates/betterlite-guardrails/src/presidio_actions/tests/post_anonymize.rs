#![cfg(test)]

use std::time::Duration;

use axum::http::StatusCode;
use serde_json::value::RawValue;

use super::shared::{
    anonymize_requests, anonymizer, item, json_reply, json_reply_with, plain_reply, seen_item,
    spawn_anonymizer,
};
use crate::presidio_actions::anonymize::{post_anonymize, read_body};
use crate::presidio_actions::error::PresidioActionsError;

#[tokio::test]
async fn posts_text_and_analyzer_results_to_the_anonymize_endpoint() {
    let mock = spawn_anonymizer(
        |_| {
            json_reply(
                r#"{"text":"MASKED","items":[{"entity_type":"PERSON"},{"entity_type":null},{}]}"#,
            )
        },
        Duration::ZERO,
    )
    .await;
    let response = post_anonymize(&anonymizer(&mock.base), "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap();
    assert_eq!(response.text, "MASKED");
    assert_eq!(response.items.len(), 3);
    assert_eq!(response.items[0].entity_type.as_deref(), Some("PERSON"));
    assert_eq!(response.items[1].entity_type, None);
    assert_eq!(response.items[2].entity_type, None);
    let requests = anonymize_requests(&mock);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].text, "Alice");
    assert_eq!(requests[0].analyzer_results.len(), 1);
    let detection = seen_item(&requests[0].analyzer_results[0]);
    assert_eq!(detection.entity_type.as_deref(), Some("PERSON"));
    assert_eq!((detection.start, detection.end), (Some(0), Some(5)));
    assert_eq!(detection.score, Some(1.0_f64));
}

#[tokio::test]
async fn analyzer_fields_beyond_the_typed_ones_reach_the_anonymizer() {
    let mut detection = item("PERSON", 0, 5);
    detection.other.insert(
        "analysis_explanation".to_owned(),
        RawValue::from_string(r#"{"pattern":"name"}"#.to_owned()).unwrap(),
    );
    let mock = spawn_anonymizer(|_| json_reply(r#"{"text":"MASKED"}"#), Duration::ZERO).await;
    post_anonymize(&anonymizer(&mock.base), "Alice", &[detection])
        .await
        .unwrap();
    let requests = anonymize_requests(&mock);
    let forwarded = seen_item(&requests[0].analyzer_results[0]);
    assert!(forwarded.analysis_explanation.is_some());
}

#[tokio::test]
async fn an_http_error_returns_status_and_a_200_character_sample() {
    let mock = spawn_anonymizer(
        |_| (StatusCode::BAD_REQUEST, "application/json", "e".repeat(300)),
        Duration::ZERO,
    )
    .await;
    let error = post_anonymize(&anonymizer(&mock.base), "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap_err();
    let PresidioActionsError::AnonymizerHttpStatus { status, body } = error else {
        panic!("expected an HTTP status error");
    };
    assert_eq!(status, 400);
    assert_eq!(body.chars().count(), 200);
}

#[tokio::test]
async fn an_upper_case_json_content_type_is_accepted() {
    let mock = spawn_anonymizer(
        |_| {
            (
                StatusCode::OK,
                "Application/JSON",
                r#"{"text":"MASKED"}"#.to_owned(),
            )
        },
        Duration::ZERO,
    )
    .await;
    let response = post_anonymize(&anonymizer(&mock.base), "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap();
    assert_eq!(response.text, "MASKED");
}

#[tokio::test]
async fn a_non_json_content_type_is_an_error() {
    let mock = spawn_anonymizer(|_| plain_reply(StatusCode::OK, "not json"), Duration::ZERO).await;
    let error = post_anonymize(&anonymizer(&mock.base), "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap_err();
    let PresidioActionsError::AnonymizerContentType { content_type, body } = error else {
        panic!("expected a content-type error");
    };
    assert_eq!(content_type, "text/plain");
    assert_eq!(body, "not json");
}

#[tokio::test]
async fn an_unparsable_body_is_an_error_without_the_original_text() {
    let mock = spawn_anonymizer(|_| json_reply("not json"), Duration::ZERO).await;
    let error = post_anonymize(
        &anonymizer(&mock.base),
        "SECRET Alice",
        &[item("PERSON", 6, 11)],
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        PresidioActionsError::AnonymizerResponseJson { .. }
    ));
    assert!(!error.to_string().contains("SECRET"));
}

#[tokio::test]
async fn a_response_without_text_is_an_error() {
    let mock = spawn_anonymizer(|_| json_reply(r#"{"items":[]}"#), Duration::ZERO).await;
    let error = post_anonymize(&anonymizer(&mock.base), "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PresidioActionsError::AnonymizerResponseJson { .. }
    ));
}

#[tokio::test]
async fn a_slow_anonymizer_hits_the_configured_timeout() {
    let mock = spawn_anonymizer(
        |_| json_reply(r#"{"text":"MASKED"}"#),
        Duration::from_millis(300),
    )
    .await;
    let mut client = anonymizer(&mock.base);
    client.settings.timeout = Duration::from_millis(20);
    let error = post_anonymize(&client, "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap_err();
    let PresidioActionsError::AnonymizerRequest { source } = error else {
        panic!("expected a request error");
    };
    assert!(source.is_timeout());
}

#[tokio::test]
async fn a_response_body_past_the_limit_is_refused() {
    let mock = spawn_anonymizer(|_| json_reply(r#"{"text":"0123456789"}"#), Duration::ZERO).await;
    let response = reqwest::Client::default()
        .post(format!("{}/anonymize", mock.base))
        .send()
        .await
        .unwrap();
    let error = read_body(response, 4).await.unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::ResponseTooLarge { limit: 4 }
    ));
}

#[tokio::test]
async fn a_payload_the_anonymizer_rejects_with_a_server_error_keeps_the_status() {
    let mock = spawn_anonymizer(
        |_| json_reply_with(StatusCode::SERVICE_UNAVAILABLE, r#"{"error":"down"}"#),
        Duration::ZERO,
    )
    .await;
    let error = post_anonymize(&anonymizer(&mock.base), "Alice", &[item("PERSON", 0, 5)])
        .await
        .unwrap_err();
    let PresidioActionsError::AnonymizerHttpStatus { status, body } = error else {
        panic!("expected an HTTP status error");
    };
    assert_eq!(status, 503);
    assert_eq!(body, r#"{"error":"down"}"#);
}
