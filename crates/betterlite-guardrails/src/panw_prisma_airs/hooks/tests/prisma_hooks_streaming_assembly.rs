use axum::http::StatusCode;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;

use super::support::{
    self, LogCapture, data, field, guardrail, json, no_masking, response, response_masking,
    settings, text_of,
};
use crate::panw_prisma_airs::error_detail::ErrorCode;
use crate::panw_prisma_airs::hooks::streaming::StreamError;
use crate::panw_prisma_airs::hooks::streaming::assembly::scan_streaming_response;
use crate::panw_prisma_airs::verdict::{Action, Failure};
use crate::prisma_airs_masking::{MaskContent, MaskedResponseMessage};

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const BLOCK: &str = r#"{"action":"block","category":"toxic","scan_id":"scan-2"}"#;
const MASKED: &str =
    r#"{"action":"allow","category":"benign","response_masked_data":{"data":"MASKED"}}"#;
const BLOCK_MASKED: &str =
    r#"{"action":"block","category":"toxic","response_masked_data":{"data":"MASKED"}}"#;
const ANSWER: &str = r#"{"choices":[{"message":{"content":"answer"}}]}"#;

fn content_text(message: &MaskedResponseMessage) -> String {
    match message.content.as_ref() {
        Some(MaskContent::Text(text)) => text.clone(),
        _ => panic!("text content"),
    }
}

#[tokio::test]
async fn an_allowed_response_is_not_modified() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert!(!scan.modified);
    assert_eq!(scan.verdict.action, Action::Allow);
    assert_eq!(scan.verdict.category.as_deref(), Some("benign"));
    let choices = model_response.choices.as_ref().expect("choices");
    let message = choices
        .first()
        .expect("choice")
        .message
        .as_ref()
        .expect("message");
    assert_eq!(content_text(message), "answer");
}

#[tokio::test]
async fn an_allowed_mask_modifies_every_choice() {
    let logs = LogCapture::start("assembly-mask", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, MASKED).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response =
        response(r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}}]}"#);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert!(scan.modified);
    let choices = model_response.choices.as_ref().expect("choices");
    let first = choices
        .first()
        .expect("choice")
        .message
        .as_ref()
        .expect("message");
    assert_eq!(content_text(first), "MASKED");
    let second = choices
        .get(1)
        .expect("choice")
        .message
        .as_ref()
        .expect("message");
    assert_eq!(content_text(second), "MASKED");
    let written = logs.finish();
    assert!(
        written.contains(
            "PANW Prisma AIRS: Streaming response allowed with masking (Category: benign)"
        )
    );
}

#[tokio::test]
async fn a_blocked_response_with_a_mask_but_no_flag_errors() {
    let server = support::serve(StatusCode::OK, BLOCK_MASKED).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let error = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect_err("blocked");
    match error {
        StreamError::Http(http) => {
            assert_eq!(http.status, 400);
            assert_eq!(
                http.detail.error.code,
                ErrorCode::PanwPrismaAirsResponseBlocked
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    let choices = model_response.choices.as_ref().expect("choices");
    let message = choices
        .first()
        .expect("choice")
        .message
        .as_ref()
        .expect("message");
    assert_eq!(content_text(message), "answer");
}

#[tokio::test]
async fn a_blocked_response_with_a_mask_and_the_flag_is_masked() {
    let logs = LogCapture::start("assembly-block-mask", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, BLOCK_MASKED).await;
    let guardrail = guardrail(settings(
        &server.base,
        response_masking(),
        FallbackAction::Block,
    ));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("masked");
    assert!(scan.modified);
    let choices = model_response.choices.as_ref().expect("choices");
    let message = choices
        .first()
        .expect("choice")
        .message
        .as_ref()
        .expect("message");
    assert_eq!(content_text(message), "MASKED");
    let written = logs.finish();
    assert!(written.contains("Response blocked but masked instead (mask_response_content=True)"));
}

#[tokio::test]
async fn empty_content_is_a_synthetic_allow() {
    let logs = LogCapture::start("assembly-empty", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(r#"{"choices":[]}"#);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert!(!scan.modified);
    assert_eq!(scan.verdict.action, Action::Allow);
    assert_eq!(scan.verdict.category.as_deref(), Some("no_content"));
    assert!(server.body_texts().await.is_empty());
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS: No content to scan in streaming response"));
}

#[tokio::test]
async fn whitespace_content_is_no_content() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(r#"{"choices":[{"message":{"content":"   "}}]}"#);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert_eq!(scan.verdict.category.as_deref(), Some("no_content"));
    assert!(server.body_texts().await.is_empty());
}

#[tokio::test]
async fn a_transient_failure_is_returned_unmodified() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert!(!scan.modified);
    assert_eq!(scan.verdict.failure, Some(Failure::Transient));
    assert_eq!(scan.verdict.category.as_deref(), Some("http_429_error"));
}

#[tokio::test]
async fn a_permanent_failure_is_returned_unmodified() {
    let server = support::serve(StatusCode::BAD_REQUEST, "{}").await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert!(!scan.modified);
    assert_eq!(scan.verdict.failure, Some(Failure::Permanent));
}

#[tokio::test]
async fn tool_call_arguments_are_scanned() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(
        r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"a\":1}"}}]}}]}"#,
    );
    let scan = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect("scan");
    assert!(!scan.modified);
    let bodies = server.body_texts().await;
    let body = json(bodies.first().expect("one scan"));
    let contents = support::array_of(field(&body, "contents"));
    assert_eq!(
        text_of(field(contents.first().expect("content"), "response")),
        "{\"a\":1}"
    );
}

#[tokio::test]
async fn an_oversized_mask_errors() {
    let masked = "x".repeat(1024 * 1024 + 1);
    let body = format!(
        r#"{{"action":"allow","category":"benign","response_masked_data":{{"data":"{masked}"}}}}"#
    );
    let server = support::serve(StatusCode::OK, &body).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let error = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect_err("oversized");
    assert!(matches!(error, StreamError::Masking(_)));
}

#[tokio::test]
async fn unreadable_metadata_errors_before_the_scan() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1","metadata":"x"}"#);
    let mut model_response = response(ANSWER);
    let error = scan_streaming_response(
        &guardrail,
        &request_data,
        &mut model_response,
        support::now(),
    )
    .await
    .expect_err("metadata");
    assert!(matches!(error, StreamError::Data { .. }));
    assert!(server.body_texts().await.is_empty());
}
