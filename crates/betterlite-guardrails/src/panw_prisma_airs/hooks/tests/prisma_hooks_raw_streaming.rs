use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;

use super::support::{
    self, LogCapture, data, guardrail, json, no_masking, response_masking, settings,
};
use crate::panw_prisma_airs::audit::{GuardrailAudit, GuardrailStatus, Stage};
use crate::panw_prisma_airs::error_detail::ErrorCode;
use crate::panw_prisma_airs::hooks::streaming::StreamChunk;
use crate::panw_prisma_airs::hooks::streaming::StreamError;
use crate::panw_prisma_airs::hooks::streaming::raw::{
    extract_event_text, extract_text_from_sse_bytes, scan_raw_streaming_text,
};
use crate::panw_prisma_airs::verdict::Action;

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const BLOCK: &str = r#"{"action":"block","category":"toxic","scan_id":"scan-2"}"#;
const BLOCK_MASK: &str =
    r#"{"action":"block","category":"toxic","response_masked_data":{"data":"MASKED"}}"#;
const MASK_WARNING: &str = "PANW Prisma AIRS: mask_response_content is configured but cannot be applied to raw streaming responses (/v1/messages or /v1/responses). Blocking response instead.";

fn sse(source: &str) -> StreamChunk {
    StreamChunk::Bytes(source.as_bytes().to_vec())
}

fn event(source: &str) -> StreamChunk {
    StreamChunk::Event(json(source))
}

#[test]
fn sse_bytes_join_text_deltas() {
    let chunks = vec![
        sse("data: {\"type\":\"message_start\"}\n\n"),
        sse(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
        ),
        sse(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"x\",\"text\":\"x\"}}\n\n",
        ),
    ];
    assert_eq!(extract_text_from_sse_bytes(&chunks), "Hello");
}

#[rstest::rstest]
#[case::a_raw_line_separator_stays_in_the_text("a\u{2028}b", "a\u{2028}b")]
#[case::a_raw_next_line_stays_in_the_text("a\u{85}b", "a\u{85}b")]
#[case::a_raw_paragraph_separator_stays_in_the_text("a\u{2029}b", "a\u{2029}b")]
fn sse_lines_split_on_newline_only(#[case] text: &str, #[case] expected: &str) {
    let line = format!(
        "data: {{\"type\":\"content_block_delta\",\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}\n\n"
    );
    assert_eq!(extract_text_from_sse_bytes(&[sse(&line)]), expected);
}

#[rstest::rstest]
#[case::crlf_line_endings("", "\r")]
#[case::a_leading_file_separator("\u{1c}", "")]
#[case::a_trailing_unit_separator("", "\u{1f}")]
#[case::a_leading_space(" ", "")]
fn sse_lines_strip_python_whitespace(#[case] before: &str, #[case] after: &str) {
    let line = format!(
        "{before}data: {{\"type\":\"content_block_delta\",\"delta\":{{\"type\":\"text_delta\",\"text\":\"x\"}}}}{after}\n"
    );
    assert_eq!(extract_text_from_sse_bytes(&[sse(&line)]), "x");
}

#[rstest::rstest]
#[case::a_delta_that_is_not_an_object(r#"{"type":"content_block_delta","delta":"x"}"#)]
#[case::a_text_that_is_not_a_string(
    r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":1}}"#
)]
#[case::a_payload_that_is_not_an_object(r#"["content_block_delta"]"#)]
fn a_malformed_sse_delta_is_skipped(#[case] payload: &str) {
    let ok = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n";
    let chunks = vec![sse(&format!("data: {payload}\n")), sse(ok)];
    assert_eq!(extract_text_from_sse_bytes(&chunks), "ok");
}

#[test]
fn an_sse_data_line_without_the_space_is_skipped() {
    let chunks = vec![sse(
        "data:{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"x\"}}\n\n",
    )];
    assert_eq!(extract_text_from_sse_bytes(&chunks), "");
}

#[test]
fn invalid_utf8_in_an_sse_line_becomes_one_replacement_character() {
    let mut bytes =
        b"data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"a"
            .to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(b"b\"}}\n\n");
    assert_eq!(
        extract_text_from_sse_bytes(&[StreamChunk::Bytes(bytes)]),
        "a\u{FFFD}b"
    );
}

#[test]
fn an_sse_event_split_across_byte_chunks_is_joined_first() {
    let chunks = vec![
        sse("data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"te"),
        sse("xt_delta\",\"text\":\"joined\"}}\n\n"),
    ];
    assert_eq!(extract_text_from_sse_bytes(&chunks), "joined");
}

#[test]
fn event_text_reads_output_text_deltas() {
    let chunks = vec![
        event(r#"{"type":"response.created"}"#),
        event(r#"{"type":"response.output_text.delta","delta":"a"}"#),
        event(r#"{"type":"response.output_text.delta","delta":"b"}"#),
    ];
    assert_eq!(extract_event_text(&chunks), "ab");
}

#[test]
fn event_text_reads_chat_completion_deltas() {
    let chunks = vec![event(
        r#"{"object":"chat.completion.chunk","choices":[{"delta":{"content":"x"}},{"delta":{}}]}"#,
    )];
    assert_eq!(extract_event_text(&chunks), "x");
}

#[test]
fn event_text_falls_back_to_the_done_text() {
    let chunks = vec![
        event(r#"{"type":"response.created","text":"x"}"#),
        event(r#"{"type":"response.output_text.done","text":"full"}"#),
    ];
    assert_eq!(extract_event_text(&chunks), "full");
}

#[test]
fn event_text_prefers_deltas_over_the_done_text() {
    let chunks = vec![
        event(r#"{"type":"response.output_text.delta","delta":"a"}"#),
        event(r#"{"type":"response.output_text.done","text":"full"}"#),
    ];
    assert_eq!(extract_event_text(&chunks), "a");
}

#[tokio::test]
async fn blank_text_skips_the_scan() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "  \n",
        &mut audit,
        support::now(),
    )
    .await
    .expect("skipped");
    assert!(server.body_texts().await.is_empty());
    assert!(audit.information.is_empty());
    assert!(audit.applied_guardrails.is_empty());
    assert!(audit.scan_ids.is_empty());
}

#[tokio::test]
async fn allow_records_the_raw_response_scan() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
        support::now(),
    )
    .await
    .expect("allowed");
    let information = audit.information.first().expect("information");
    assert_eq!(information.status, GuardrailStatus::Success);
    assert_eq!(information.event_type, Stage::PostCall);
    assert_eq!(information.response.action, Action::Allow);
    assert_eq!(information.response.category.as_deref(), Some("benign"));
    assert_eq!(information.start_time, support::now());
    assert_eq!(information.end_time, support::now());
    assert_eq!(information.duration, Duration::ZERO);
    assert_eq!(audit.scan_ids, vec!["scan-1".to_owned()]);
    let entry = audit.scan_metadata.first().expect("scan entry");
    assert_eq!(entry.stage, Stage::PostCall);
    assert_eq!(entry.scan_id, "scan-1");
    assert_eq!(audit.applied_guardrails, vec!["panw".to_owned()]);
    let body = support::only_scan_body(&server).await;
    assert_eq!(support::scanned_text(&body, "response"), "hello");
}

#[tokio::test]
async fn block_returns_the_400_without_recording_a_scan() {
    let server = support::serve(StatusCode::OK, BLOCK).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    let error = scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
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
    assert!(audit.information.is_empty());
    assert!(audit.applied_guardrails.is_empty());
    assert!(audit.scan_ids.is_empty());
}

#[tokio::test]
async fn a_mask_with_the_response_flag_warns_and_blocks() {
    let logs = LogCapture::start("raw-mask-warn", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, BLOCK_MASK).await;
    let guardrail = guardrail(settings(
        &server.base,
        response_masking(),
        FallbackAction::Block,
    ));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    let error = scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
        support::now(),
    )
    .await
    .expect_err("blocked");
    match error {
        StreamError::Http(http) => assert_eq!(http.status, 400),
        other => panic!("unexpected {other:?}"),
    }
    let written = logs.finish();
    assert!(written.contains(MASK_WARNING));
}

#[tokio::test]
async fn a_mask_without_the_response_flag_logs_no_warning() {
    let logs = LogCapture::start("raw-mask-quiet", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, BLOCK_MASK).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
        support::now(),
    )
    .await
    .expect_err("blocked");
    let written = logs.finish();
    assert!(!written.contains("mask_response_content is configured"));
}

#[tokio::test]
async fn transient_error_with_fallback_allow_marks_the_response_unscanned() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Allow));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
        support::now(),
    )
    .await
    .expect("unscanned");
    assert_eq!(audit.applied_guardrails, vec!["panw:unscanned".to_owned()]);
    let information = audit.information.first().expect("information");
    assert_eq!(
        information.status,
        GuardrailStatus::GuardrailFailedToRespond
    );
    assert_eq!(
        information.response.category.as_deref(),
        Some("http_429_error")
    );
    assert!(audit.scan_ids.is_empty());
}

#[tokio::test]
async fn transient_error_with_fallback_block_returns_a_500() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut audit = GuardrailAudit::default();
    let error = scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
        support::now(),
    )
    .await
    .expect_err("blocked");
    match error {
        StreamError::Http(http) => {
            assert_eq!(http.status, 500);
            assert_eq!(http.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn unreadable_metadata_returns_a_data_error() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let request_data = data(r#"{"litellm_call_id":"call-1","metadata":"x"}"#);
    let mut audit = GuardrailAudit::default();
    let error = scan_raw_streaming_text(
        &guardrail,
        &request_data,
        "hello",
        &mut audit,
        support::now(),
    )
    .await
    .expect_err("metadata");
    match error {
        StreamError::Data { source } => assert!(
            source
                .to_string()
                .contains("the request metadata could not be read")
        ),
        other => panic!("unexpected {other:?}"),
    }
    assert!(server.body_texts().await.is_empty());
}
