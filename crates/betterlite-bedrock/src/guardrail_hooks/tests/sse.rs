use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use betterlite_core::routes::CallType;

use super::fake::base;
use super::support::{
    chat_response, frame_texts, object, routes, run_buffered, sse_frame, streaming_settings, text,
};
use crate::guardrail_hooks::chunks::StreamChunk;
use crate::guardrail_hooks::errors::HookFailure;
use crate::guardrail_hooks::streaming::{StreamHook, streaming_iterator_hook};

fn raw_chunks() -> Vec<StreamChunk> {
    vec![
        sse_frame(
            "message_start",
            "{\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-test\"}}",
        ),
        sse_frame(
            "content_block_delta",
            "{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}",
        ),
    ]
}

#[tokio::test]
async fn masks_and_rewrites_raw_frames() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.scan_result = Ok(object(vec![]));
    runtime.masked_response = Some(chat_response("claude-test", text("masked")));
    runtime.sse_frames = vec![b"event: message_start\ndata: {}\n\n".to_vec()];
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap();
    assert_eq!(
        output,
        vec![StreamChunk::Bytes(runtime.sse_frames[0].clone())]
    );
    assert_eq!(runtime.masked.lock().unwrap().len(), 1);
    assert_eq!(runtime.scans.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn keeps_the_original_chunks_when_the_text_did_not_change() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.scan_result = Ok(object(vec![]));
    runtime.masked_response = Some(chat_response("claude-test", text("hello")));
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap();
    assert_eq!(output, raw_chunks());
}

#[tokio::test]
async fn an_unassemblable_raw_stream_becomes_an_error_frame() {
    let runtime = base();
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap();
    let frames = frame_texts(&output);
    assert_eq!(frames.len(), 1);
    assert!(frames[0].contains("bedrock-guard"));
    assert!(frames[0].contains("could not be assembled for scanning"));
}

#[tokio::test]
async fn a_structured_400_block_becomes_an_error_frame() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 400,
        detail: object(vec![("error", text("blocked by policy"))]),
    });
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap();
    let frames = frame_texts(&output);
    assert!(frames[0].contains("blocked by policy"));
    assert!(!frames[0].contains("400:"));
}

#[tokio::test]
async fn a_service_failure_after_a_keepalive_ping_becomes_a_frame() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.elapsed = Duration::from_secs(2);
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 500,
        detail: text("bedrock is down"),
    });
    let mut settings = streaming_settings();
    settings.keepalive_interval_seconds = Some(1.0_f64);
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap();
    let frames = frame_texts(&output);
    assert!(frames[0].contains("500: bedrock is down"));
}

#[tokio::test]
async fn a_service_failure_before_a_keepalive_ping_raises() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.elapsed = Duration::from_millis(500);
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 500,
        detail: text("bedrock is down"),
    });
    let mut settings = streaming_settings();
    settings.keepalive_interval_seconds = Some(1.0_f64);
    let failure = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap_err();
    match failure {
        HookFailure::Rejected { status, .. } => assert_eq!(status, 500),
        other @ (HookFailure::ModifyResponse(_) | HookFailure::Failed { .. }) => {
            panic!("expected the service failure to raise, got {other:?}");
        }
    }
}

#[tokio::test]
async fn a_service_failure_with_pings_off_raises() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.elapsed = Duration::from_secs(30);
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 500,
        detail: text("bedrock is down"),
    });
    let settings = streaming_settings();
    let failure = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap_err();
    assert_eq!(
        failure,
        HookFailure::Rejected {
            status: 500,
            detail: text("bedrock is down"),
        }
    );
}

#[tokio::test]
async fn a_plain_400_string_detail_is_a_service_failure_not_a_block() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 400,
        detail: text("bad request"),
    });
    let settings = streaming_settings();
    let failure = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap_err();
    assert_eq!(
        failure,
        HookFailure::Rejected {
            status: 400,
            detail: text("bad request"),
        }
    );
}

#[tokio::test]
async fn a_service_failure_with_a_structured_detail_raises_until_a_ping_fires() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 500,
        detail: object(vec![("error", text("bedrock is down"))]),
    });
    let settings = streaming_settings();
    let failure = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap_err();
    match failure {
        HookFailure::Rejected { status, .. } => assert_eq!(status, 500),
        other @ (HookFailure::ModifyResponse(_) | HookFailure::Failed { .. }) => {
            panic!("expected the service failure to raise, got {other:?}");
        }
    }
}

#[tokio::test]
async fn the_keepalive_clock_reads_the_instant_the_caller_stamped() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("claude-test", text("hello")));
    runtime.elapsed = Duration::from_secs(2);
    runtime.scan_result = Err(HookFailure::Rejected {
        status: 500,
        detail: text("bedrock is down"),
    });
    let mut settings = streaming_settings();
    settings.keepalive_interval_seconds = Some(1.0_f64);
    let table = routes("/v1/chat/completions", vec![CallType::Completion]);
    let mut data = BTreeMap::new();
    let started = Instant::now()
        .checked_sub(Duration::from_secs(5))
        .expect("an instant five seconds ago exists");
    streaming_iterator_hook(StreamHook {
        runtime: &runtime,
        settings: &settings,
        routes: &table,
        request_route: Some("/v1/chat/completions"),
        data: &mut data,
        chunks: raw_chunks(),
        started,
    })
    .await
    .unwrap();
    assert_eq!(*runtime.since.lock().unwrap(), Some(started));
}

#[tokio::test]
async fn the_error_frame_reports_the_guardrail_error_type() {
    let runtime = base();
    let settings = streaming_settings();
    let output = run_buffered(&runtime, &settings, "/v1/chat/completions", raw_chunks())
        .await
        .unwrap();
    let frames = frame_texts(&output);
    assert!(frames[0].contains("\"guardrail_error\""));
    assert!(frames[0].starts_with("event: error\n"));
}
