#![cfg(test)]

use std::iter::repeat_with;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::{Admission, Callback as _};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::seams::{LoggingFuture, LoggingJob};
use super::{ChunkError, Logged, SseChunk, convert_chunk_to_sse_format};
use support::{Harness, at, base_time, deferred_callback, logging_object};

mod failures;
mod support;

fn mapping(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

#[tokio::test]
async fn construction_stamps_the_start_time() {
    let harness = Harness::new(None);
    assert_eq!(harness.stream.start_time, base_time());
    assert!(harness.stream.completion_start_time.is_none());
}

#[rstest]
#[case::first_chunk_seen(true, 2, Some(1))]
#[case::no_first_chunk(false, 1, None)]
#[tokio::test]
async fn logging_arguments_and_first_chunk_time(
    #[case] first_chunk: bool,
    #[case] end_seconds: i64,
    #[case] first_seconds: Option<i64>,
) {
    let mut harness = Harness::new(None);
    if first_chunk {
        harness.stream.record_first_chunk();
        harness.stream.record_first_chunk();
    }
    let chunks = vec![b"a".to_vec(), b"b".to_vec()];
    let logged = harness
        .stream
        .handle_streaming_logging(chunks.clone(), false)
        .expect("the route accepts the call");
    assert_eq!(logged, Logged::Enqueued(Admission::Accepted));
    harness.worker.flush().await;
    assert_eq!(harness.ran(), 1);
    assert_eq!(harness.worker.stats().accepted, 1);
    let routed = harness
        .seams
        .routed
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let call = routed.first().expect("the route was called once");
    assert_eq!(routed.len(), 1);
    assert_eq!(call.raw_bytes, chunks);
    assert_eq!(call.request_body, harness.request_body);
    assert_eq!(call.endpoint_type, EndpointType::Anthropic);
    assert_eq!(call.url_route, "/v1/messages");
    assert_eq!(call.start_time, base_time());
    assert_eq!(call.end_time, at(end_seconds));
    assert!(Arc::ptr_eq(&call.logging_obj, &harness.logging_obj));
    assert!(Arc::ptr_eq(&call.success_handler, &harness.handler));
    let seen = harness
        .logging_obj
        .read(&mut |state| {
            (
                state.completion_start_time,
                state
                    .model_call_details
                    .get("completion_start_time")
                    .cloned(),
            )
        })
        .expect("the logging state is present");
    let first = first_seconds.map(at);
    assert_eq!(seen.0, first);
    assert_eq!(seen.1, first.map(|time| Json::Text(time.to_rfc3339())));
}

#[rstest]
#[case::normal_completion(false, Logged::Parked, 0, true)]
#[case::stream_teardown(true, Logged::Enqueued(Admission::Accepted), 1, false)]
#[tokio::test]
async fn deferred_billing_and_teardown(
    #[case] teardown: bool,
    #[case] expected: Logged,
    #[case] ran: usize,
    #[case] parked: bool,
) {
    let harness = Harness::new(None);
    let calls = Arc::new(Mutex::new(Vec::new()));
    harness.arm_deferred(Some(deferred_callback(&calls)), None);
    let logged = harness
        .stream
        .handle_streaming_logging(vec![b"a".to_vec()], teardown)
        .expect("the route accepts the call");
    assert_eq!(logged, expected);
    harness.worker.flush().await;
    assert_eq!(harness.ran(), ran);
    assert_eq!(harness.worker.stats().accepted, ran);
    let (args_parked, callback_present) = {
        let fields = harness
            .slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (
            fields.deferred_args.is_some(),
            fields.deferred_callback.is_some(),
        )
    };
    assert_eq!(args_parked, parked);
    assert!(callback_present);
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    // The parked future is the routed logging itself, so dispatching it runs that future
    assert_eq!(
        harness.stream.dispatch_pending_logging(),
        parked.then_some(Admission::Accepted)
    );
    harness.worker.flush().await;
    assert_eq!(harness.ran(), 1);
    assert_eq!(
        *calls.lock().unwrap_or_else(PoisonError::into_inner),
        if parked { vec![1] } else { Vec::new() }
    );
}

#[rstest]
#[case::callback_and_args(true, Some(1), Some(Admission::Accepted), vec![1])]
#[case::empty_args(true, Some(0), Some(Admission::Accepted), vec![0])]
#[case::missing_callback(false, Some(1), None, vec![])]
#[case::missing_args(true, None, None, vec![])]
#[tokio::test]
async fn deferred_dispatch_clears_both_fields(
    #[case] armed: bool,
    #[case] args: Option<usize>,
    #[case] dispatched: Option<Admission>,
    #[case] expected_calls: Vec<usize>,
) {
    let harness = Harness::new(None);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let parked: Option<Vec<LoggingFuture>> = args.map(|count| {
        repeat_with(|| -> LoggingFuture { Box::pin(async { Ok(()) }) })
            .take(count)
            .collect()
    });
    harness.arm_deferred(armed.then_some(deferred_callback(&calls)), parked);
    assert_eq!(harness.stream.dispatch_pending_logging(), dispatched);
    assert_eq!(harness.stream.dispatch_pending_logging(), None);
    harness.worker.flush().await;
    assert_eq!(
        *calls.lock().unwrap_or_else(PoisonError::into_inner),
        expected_calls
    );
    assert_eq!(
        harness.worker.stats().completed,
        usize::from(dispatched.is_some())
    );
    let (callback_left, args_left) = {
        let fields = harness
            .slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (
            fields.deferred_callback.is_some(),
            fields.deferred_args.is_some(),
        )
    };
    assert_eq!(callback_left, armed && args.is_none());
    assert_eq!(args_left, !armed && args.is_some());
}

#[rstest]
#[case::dict_without_type(r#"{"x": 1}"#, "event: message\ndata: {\"x\": 1}\n\n")]
#[case::ping(r#"{"type": "ping"}"#, "event: ping\ndata: {\"type\": \"ping\"}\n\n")]
#[case::null_type(r#"{"type": null}"#, "event: None\ndata: {\"type\": null}\n\n")]
#[case::ascii_escapes(
    r#"{"type": "\u00e9", "n": [1, 2]}"#,
    "event: \u{e9}\ndata: {\"type\": \"\\u00e9\", \"n\": [1, 2]}\n\n"
)]
fn native_chunk_format(#[case] chunk: &str, #[case] frame: &str) {
    let sse = convert_chunk_to_sse_format(SseChunk::Mapping(mapping(chunk)));
    assert_eq!(sse, Ok(frame.as_bytes().to_vec()));
}

#[rstest]
#[case::ping_frame(b"event: ping\ndata: {}\n\n".to_vec())]
#[case::empty(Vec::new())]
fn native_chunk_format_passes_bytes_unchanged(#[case] chunk: Vec<u8>) {
    assert_eq!(
        convert_chunk_to_sse_format(SseChunk::Bytes(chunk.clone())),
        Ok(chunk)
    );
}

#[rstest]
#[case::number(r#"{"type": 1}"#)]
#[case::list(r#"{"type": ["ping"]}"#)]
fn native_chunk_format_rejects_a_type_that_is_not_text(#[case] chunk: &str) {
    assert_eq!(
        convert_chunk_to_sse_format(SseChunk::Mapping(mapping(chunk))),
        Err(ChunkError::EventTypeNotText)
    );
}

#[tokio::test]
async fn direct_chunk_processor_delegation() {
    let harness = Harness::new(None);
    let supplied_obj = logging_object();
    let supplied_body = mapping(r#"{"model": "other"}"#);
    let relayed = harness.stream.streaming_response_iterator(
        "live response".to_owned(),
        supplied_body.clone(),
        Arc::clone(&supplied_obj),
    );
    assert_eq!(relayed, "processed live response");
    harness.worker.flush().await;
    let processed = harness
        .seams
        .processed
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let call = processed.first().expect("the processor was called once");
    assert_eq!(processed.len(), 1);
    assert_eq!(call.response, "live response");
    assert_eq!(call.request_body, supplied_body);
    assert!(Arc::ptr_eq(&call.logging_obj, &supplied_obj));
    assert!(!Arc::ptr_eq(&call.logging_obj, &harness.logging_obj));
    assert_eq!(call.endpoint_type, EndpointType::Anthropic);
    assert_eq!(call.start_time, base_time());
    assert_eq!(call.url_route, "/v1/messages");
    assert!(Arc::ptr_eq(&call.success_handler, &harness.handler));
    assert!(
        harness
            .seams
            .routed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert_eq!(harness.worker.stats().accepted, 0);
}

#[test]
fn logging_job_is_named_for_the_worker() {
    let job = LoggingJob {
        future: Box::pin(async { Ok(()) }),
    };
    assert_eq!(job.name(), "messages_stream_logging");
}
