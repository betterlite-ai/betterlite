use std::sync::PoisonError;

use betterlite_azure::passthrough::logging::AzureRelayedLog;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;

use crate::passthrough::logging::FoundryRelayedLog;
use crate::passthrough::tests::{
    RecordingLogging, RecordingStream, build_plan, object, plan_with_stream, target,
};

use super::ocr_cases::scripted_ocr;

const RESPONSES_CHUNK: &str = r#"data: {"type":"response.completed","sequence_number":2,"response":{"id":"resp_1","created_at":7,"output":[],"status":"completed"}}"#;

pub(super) fn stream_collected_log() {
    responses_streams_log_the_terminal_event();
    chat_streams_delegate_to_the_assembly();
    the_default_assembly_logs_no_chat_stream();
    stream_collection_skips_ocr();
}

fn responses_streams_log_the_terminal_event() {
    let chunks = vec![String::from(RESPONSES_CHUNK)];
    let plan = plan_with_stream(RecordingStream::default());
    let mut logging = RecordingLogging::default();
    let logged =
        plan.handle_collected_chunks(&chunks, &mut logging, &target("gpt-4o", "/v1/responses"));
    assert!(matches!(
        logged,
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Terminal(_)))
    ));
    assert_eq!(logging.call_type.as_deref(), Some("aresponses"));
}

fn chat_streams_delegate_to_the_assembly() {
    let chunks = vec![String::from(RESPONSES_CHUNK)];
    let stream = RecordingStream::default();
    let calls = stream.calls.clone();
    let plan = plan_with_stream(stream);
    let mut logging = RecordingLogging::default();
    let logged = plan.handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-4o", "openai/chat/completions"),
    );
    assert!(matches!(
        logged,
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Chat(_)))
    ));
    let recorded = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].chunks, 1_usize);
    assert_eq!(recorded[0].model, "gpt-4o");
    assert_eq!(recorded[0].messages, None);
}

fn the_default_assembly_logs_no_chat_stream() {
    let chunks = vec![String::from(RESPONSES_CHUNK)];
    let plan = build_plan(None, None, None, None, Object::new());
    let mut logging = RecordingLogging::default();
    let logged = plan.handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-4o", "openai/chat/completions"),
    );
    assert!(logged.is_none(), "the default assembly logs no chat stream");
}

fn stream_collection_skips_ocr() {
    let chunks = vec![String::from(RESPONSES_CHUNK)];
    let ocr = scripted_ocr();
    let mut plan = build_plan(None, None, None, None, Object::new());
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let logged = plan.handle_collected_chunks(
        &chunks,
        &mut logging,
        &target("gpt-x", "deployments/d/embeddings"),
    );
    assert!(logged.is_none());
    assert!(ocr.calls().is_empty(), "stream collection never runs OCR");
}
