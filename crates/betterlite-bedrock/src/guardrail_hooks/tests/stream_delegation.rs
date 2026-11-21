use std::collections::BTreeMap;
use std::time::Instant;

use betterlite_core::routes::CallType;
use rstest::rstest;

use super::fake::{DelegateRecord, base};
use super::support::{chat_response, object, routes, stream_chunk, streaming_settings, text};
use crate::guardrail_hooks::chunks::StreamChunk;
use crate::guardrail_hooks::streaming::{StreamHook, streaming_iterator_hook};
use crate::guardrail_hooks::streams_incrementally;

fn chunks() -> Vec<StreamChunk> {
    vec![
        StreamChunk::Json(stream_chunk("hello")),
        StreamChunk::Json(stream_chunk(" world")),
    ]
}

#[rstest]
#[case::mask_forces_buffering((true, false, true, false), false)]
#[case::unbuffered_streams_incrementally((false, false, false, false), true)]
#[case::the_default_buffers((false, true, false, false), false)]
#[case::release_on_scan_streams_incrementally((false, true, true, false), true)]
#[case::end_of_stream_only_buffers((false, true, true, true), false)]
fn streams_incrementally_matches_the_streaming_flags(
    #[case] flags: (bool, bool, bool, bool),
    #[case] expected: bool,
) {
    let (mask_response_content, buffer_until_moderated, buffer_release_on_scan, end_of_stream_only) =
        flags;
    let mut settings = streaming_settings();
    settings.mask_response_content = mask_response_content;
    settings.params.buffer_until_moderated = buffer_until_moderated;
    settings.params.buffer_release_on_scan = buffer_release_on_scan;
    settings.params.streaming_end_of_stream_only = end_of_stream_only;
    assert_eq!(streams_incrementally(&settings), expected);
}

#[tokio::test]
async fn incremental_streaming_delegates_with_the_default_buffering_off() {
    let runtime = base();
    let mut settings = streaming_settings();
    settings.params.buffer_until_moderated = false;
    let table = routes("/v1/chat/completions", vec![CallType::Completion]);
    let mut data = BTreeMap::new();
    let given = chunks();
    let output = streaming_iterator_hook(StreamHook {
        runtime: &runtime,
        settings: &settings,
        routes: &table,
        request_route: Some("/v1/chat/completions"),
        data: &mut data,
        chunks: given.clone(),
        started: Instant::now(),
    })
    .await
    .unwrap();
    assert_eq!(output, given);
    let records = runtime.delegations.lock().unwrap();
    assert_eq!(
        *records,
        vec![DelegateRecord {
            buffer_until_moderated_default: false,
            chunk_count: 2,
        }]
    );
}

#[tokio::test]
async fn release_on_scan_delegates_with_the_default_buffering_off() {
    let runtime = base();
    let mut settings = streaming_settings();
    settings.params.buffer_release_on_scan = true;
    let table = routes("/v1/chat/completions", vec![CallType::Completion]);
    let mut data = BTreeMap::new();
    let given = chunks();
    streaming_iterator_hook(StreamHook {
        runtime: &runtime,
        settings: &settings,
        routes: &table,
        request_route: None,
        data: &mut data,
        chunks: given,
        started: Instant::now(),
    })
    .await
    .unwrap();
    let records = runtime.delegations.lock().unwrap();
    assert!(!records[0].buffer_until_moderated_default);
    drop(records);
}

#[tokio::test]
async fn a_responses_route_delegates_with_buffering_on() {
    let runtime = base();
    let settings = streaming_settings();
    let table = routes("/v1/responses", vec![CallType::Responses]);
    let mut data = BTreeMap::new();
    let given = chunks();
    let output = streaming_iterator_hook(StreamHook {
        runtime: &runtime,
        settings: &settings,
        routes: &table,
        request_route: Some("/v1/responses"),
        data: &mut data,
        chunks: given.clone(),
        started: Instant::now(),
    })
    .await
    .unwrap();
    assert_eq!(output, given);
    let records = runtime.delegations.lock().unwrap();
    assert_eq!(
        *records,
        vec![DelegateRecord {
            buffer_until_moderated_default: true,
            chunk_count: 2,
        }]
    );
}

#[tokio::test]
async fn a_masked_stream_buffers_even_when_buffering_is_off() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("gpt-test", text("hello world")));
    runtime.scan_result = Ok(object(vec![]));
    let mut settings = streaming_settings();
    settings.mask_response_content = true;
    settings.params.buffer_until_moderated = false;
    let table = routes("/v1/chat/completions", vec![CallType::Completion]);
    let mut data = BTreeMap::new();
    let output = streaming_iterator_hook(StreamHook {
        runtime: &runtime,
        settings: &settings,
        routes: &table,
        request_route: Some("/v1/chat/completions"),
        data: &mut data,
        chunks: chunks(),
        started: Instant::now(),
    })
    .await
    .unwrap();
    assert!(runtime.delegations.lock().unwrap().is_empty());
    assert_eq!(runtime.scans.lock().unwrap().len(), 1);
    assert_eq!(output.len(), 1);
}

#[tokio::test]
async fn an_unknown_route_buffers_and_scans() {
    let mut runtime = base();
    runtime.assembled = Some(chat_response("gpt-test", text("hello world")));
    runtime.scan_result = Ok(object(vec![]));
    let settings = streaming_settings();
    let table = routes("/v1/responses", vec![CallType::Responses]);
    let mut data = BTreeMap::new();
    streaming_iterator_hook(StreamHook {
        runtime: &runtime,
        settings: &settings,
        routes: &table,
        request_route: Some("/v1/unknown"),
        data: &mut data,
        chunks: chunks(),
        started: Instant::now(),
    })
    .await
    .unwrap();
    assert!(runtime.delegations.lock().unwrap().is_empty());
    assert_eq!(runtime.scans.lock().unwrap().len(), 1);
}
