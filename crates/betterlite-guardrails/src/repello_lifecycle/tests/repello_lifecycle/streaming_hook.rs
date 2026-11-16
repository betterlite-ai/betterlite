use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::{
    BLOCKED_MESSAGE, RepelloAILifecycle, UNREACHABLE_MESSAGE,
};
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::support::{
    BLOCKED_REPLY, CapturedLog, FLAGGED_REPLY, PASSED_REPLY, applied, captured_body, captured_path,
    chat_chunks, guardrail_with, mock_repello, params, prompt_data, records, request_count,
    run_stream, settings, text_chunks, text_field, value,
};

fn guarding(base: &str, mode: &str) -> RepelloAILifecycle {
    let mut configured = params();
    configured.mode = LitellmParamsMode::One(mode.to_owned());
    guardrail_with(&configured, settings(base))
}

fn expected(chunks: &[&str]) -> Vec<GuardrailValue> {
    chunks.iter().map(|chunk| value(chunk)).collect()
}

#[tokio::test]
async fn every_chunk_is_buffered_until_the_scan_passes() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let chunks = chat_chunks();
    let run = run_stream(&guarding(&mock.base, "post_call"), prompt_data(), &chunks).await;
    assert!(run.result.is_ok());
    assert_eq!(run.output, expected(&chunks));
    assert_eq!(captured_path(&mock), "/analyze/response");
    let body = captured_body(&mock);
    let Some(GuardrailValue::Object(scan_data)) = body.get("scan_data") else {
        panic!("expected scan_data");
    };
    assert_eq!(
        text_field(&GuardrailValue::Object(scan_data.clone()), "response").as_deref(),
        Some("hello world")
    );
    assert_eq!(
        applied(&run.data),
        vec![GuardrailValue::Text(String::from("repello-test"))]
    );
    assert_eq!(
        text_field(&records(&run.data)[0], "guardrail_status").as_deref(),
        Some("success")
    );
}

#[tokio::test]
async fn a_blocked_stream_raises_before_any_chunk_is_sent() {
    let mock = mock_repello(StatusCode::OK, BLOCKED_REPLY).await;
    let run = run_stream(
        &guarding(&mock.base, "post_call"),
        prompt_data(),
        &chat_chunks(),
    )
    .await;
    let Err(error) = run.result else {
        panic!("a blocked stream must fail");
    };
    assert_eq!(error.to_string(), BLOCKED_MESSAGE);
    assert_eq!(BLOCKED_MESSAGE, "Blocked by RepelloAI Argus guardrail");
    assert!(run.output.is_empty());
    assert!(applied(&run.data).is_empty());
    assert_eq!(
        text_field(&records(&run.data)[0], "guardrail_status").as_deref(),
        Some("guardrail_intervened")
    );
}

#[tokio::test]
async fn a_flagged_stream_is_allowed_and_logged() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, FLAGGED_REPLY).await;
    let chunks = chat_chunks();
    let run = run_stream(&guarding(&mock.base, "post_call"), prompt_data(), &chunks).await;
    assert!(run.result.is_ok());
    assert!(
        captured
            .text()
            .contains("RepelloAI Argus flagged content (allowed):"),
        "{}",
        captured.text()
    );
    assert_eq!(run.output, expected(&chunks));
    assert_eq!(applied(&run.data).len(), 1);
}

#[tokio::test]
async fn a_text_completion_stream_is_not_scanned_and_is_replayed() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let chunks = text_chunks();
    let run = run_stream(&guarding(&mock.base, "post_call"), prompt_data(), &chunks).await;
    assert!(run.result.is_ok());
    assert_eq!(request_count(&mock), 0);
    assert_eq!(run.output, expected(&chunks));
    let log = captured.text();
    assert!(
        log.contains("RepelloAI Argus: no inspectable text in streamed response; skipping scan."),
        "{log}"
    );
    assert!(log.contains("assembled_type=TextResponse"), "{log}");
}

#[tokio::test]
async fn an_empty_stream_is_not_scanned() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_stream(&guarding(&mock.base, "post_call"), prompt_data(), &[]).await;
    assert!(run.result.is_ok());
    assert_eq!(request_count(&mock), 0);
    assert!(run.output.is_empty());
    assert!(
        captured.text().contains("assembled_type=NoneType"),
        "{}",
        captured.text()
    );
}

#[tokio::test]
async fn a_mode_without_post_call_relays_every_chunk_without_scanning() {
    let captured = CapturedLog::default();
    let _guard = set_default(captured.subscriber(tracing::Level::WARN));
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let chunks = text_chunks();
    let run = run_stream(&guarding(&mock.base, "pre_call"), prompt_data(), &chunks).await;
    assert!(run.result.is_ok());
    assert_eq!(request_count(&mock), 0);
    assert_eq!(run.output, expected(&chunks));
    assert_eq!(captured.text(), "");
}

#[tokio::test]
async fn a_chunk_that_is_not_a_stream_chunk_fails_the_scan() {
    let mock = mock_repello(StatusCode::OK, PASSED_REPLY).await;
    let run = run_stream(
        &guarding(&mock.base, "post_call"),
        prompt_data(),
        &["\"nope\""],
    )
    .await;
    let Err(error) = run.result else {
        panic!("a chunk that is not a chunk must fail the stream scan");
    };
    assert_eq!(
        error.to_string(),
        "RepelloAI Argus stream scan failed: a streamed chunk could not be read"
    );
    assert!(run.output.is_empty());
    assert_eq!(request_count(&mock), 0);
}

#[rstest]
#[case::fail_closed(UnreachableFallback::FailClosed, false)]
#[case::fail_open(UnreachableFallback::FailOpen, true)]
#[tokio::test]
async fn an_unreachable_api_decides_the_stream_by_the_fallback(
    #[case] fallback: UnreachableFallback,
    #[case] relayed: bool,
) {
    let mock = mock_repello(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let mut configured = params();
    configured.mode = LitellmParamsMode::One(String::from("post_call"));
    configured.unreachable_fallback = fallback;
    let chunks = chat_chunks();
    let run = run_stream(
        &guardrail_with(&configured, settings(&mock.base)),
        prompt_data(),
        &chunks,
    )
    .await;
    assert_eq!(run.result.is_ok(), relayed);
    if relayed {
        assert_eq!(run.output, expected(&chunks));
    } else {
        assert!(run.output.is_empty());
        let Err(GuardrailFailure::HttpObject { detail, .. }) = run.result else {
            panic!("expected an unreachable failure");
        };
        assert_eq!(
            detail.get("error"),
            Some(&GuardrailValue::Text(String::from(UNREACHABLE_MESSAGE)))
        );
    }
}
