use std::sync::Arc;

use betterlite_error::error::ErrorClass;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::live::{Event, LiveStream, StreamEngine};
use super::{
    BAD_REQUEST, COMPLETED, CREATED, CREATED_AT, CountingSource, DELTA, FailurePort, Fixture,
    IN_PROGRESS, LATE, Ports, QUEUED, RATE_LIMITED, RecordingCache, ReplacingHook, SERVER_ERROR,
    START, SuccessPort, caching_handler, class_of, connection_message, delta, event_type, fallback,
    fixed_clock, payload, poll_count, prefetched, scripted_clock, timeout_details, timestamp,
};
use crate::api::responses_stream_events::{BaseStreamStateInput, LoggingSnapshot, StreamTimes};
use crate::api::responses_stream_hooks::failure::await_pending_logging;
use crate::api::responses_stream_hooks::ports::Dispatch;
#[rstest]
#[case::rate_limit(RATE_LIMITED, 429, true)]
#[case::server_error(SERVER_ERROR, 500, true)]
#[case::bad_request(BAD_REQUEST, 400, false)]
#[tokio::test]
async fn terminal_error_events_raise_before_the_deployment_hook(
    #[case] event: &str,
    #[case] status: u16,
    #[case] falls_back: bool,
) {
    let hook =
        ReplacingHook::new(serde_json::from_str(COMPLETED).expect("the replacement decodes"));
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::clone(&failure));
    ports.chunk_hooks = vec![Arc::clone(&hook)];
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED), payload(event)]), None);
    stream.next_event().await.expect("the created event yields");
    let error = stream
        .next_event()
        .await
        .expect_err("the error event raises");
    assert_eq!(status, class_of(&error).metadata().status_code);
    match class_of(&error) {
        ErrorClass::MidStreamFallback(details) => {
            assert!(falls_back);
            assert_eq!(details.original_status.0, status);
        }
        other => assert!(!falls_back, "unexpected class {other:?}"),
    }
    assert_eq!(hook.calls(), vec!["response.created".to_owned()]);
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn the_deployment_hook_decides_whether_output_started() {
    let replacement: Event = serde_json::from_str(DELTA).expect("the replacement decodes");
    let hook = ReplacingHook::new(replacement);
    let mut ports = Ports::default();
    ports.chunk_hooks = vec![Arc::clone(&hook)];
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED)]), None);
    let yielded = stream
        .next_event()
        .await
        .expect("the replaced event yields")
        .expect("an event is returned");
    assert_eq!(delta(&yielded), "hi");
    let error = stream.next_event().await.expect_err("the stream fails");
    connection_message(&error);
    assert_eq!(hook.calls(), vec!["response.created".to_owned()]);
}

#[tokio::test]
async fn a_queued_event_does_not_start_the_output() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(QUEUED)]), None);
    let queued = stream
        .next_event()
        .await
        .expect("the queued event yields")
        .expect("an event is returned");
    assert_eq!(event_type(&queued), "response.queued");
    let error = stream.next_event().await.expect_err("the stream fails");
    assert_eq!(fallback(&error).original_status.0, 500);
}

#[tokio::test]
async fn an_in_progress_event_does_not_start_the_output() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(IN_PROGRESS)]), None);
    let started = stream
        .next_event()
        .await
        .expect("the event yields")
        .expect("an event is returned");
    assert_eq!(event_type(&started), "response.in_progress");
    let error = stream.next_event().await.expect_err("the stream fails");
    assert_eq!(fallback(&error).original_status.0, 500);
}

#[tokio::test]
async fn undecodable_payloads_are_skipped() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = fixture.stream(
        prefetched(vec![payload("not json"), payload(COMPLETED)]),
        None,
    );
    let event = stream
        .next_event()
        .await
        .expect("the terminal event yields")
        .expect("an event is returned");
    assert_eq!(event_type(&event), "response.completed");
}

#[tokio::test]
async fn streaming_duration_limit_blocks_the_first_read() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::clone(&failure));
    let fixture = Fixture::new(ports, scripted_clock(&[LATE]));
    let (source, polls) = CountingSource::new(&[CREATED]);
    let mut stream = fixture.stream(Box::new(source), Some(10.0));
    let error = stream.next_event().await.expect_err("the stream times out");
    assert_eq!(
        timeout_details(&error).context.message,
        "litellm.Timeout: Stream exceeded max streaming duration of 10s (elapsed 20.0s)"
    );
    assert_eq!(poll_count(&polls), 0);
    assert_eq!(failure.classes().len(), 1);
}

#[tokio::test]
async fn streaming_duration_limit_stops_after_a_received_event() {
    let fixture = Fixture::new(Ports::default(), scripted_clock(&[START, LATE]));
    let (source, polls) = CountingSource::new(&[CREATED]);
    let mut stream = fixture.stream(Box::new(source), Some(10.0));
    let error = stream.next_event().await.expect_err("the stream times out");
    assert_eq!(timeout_details(&error).original_status.0, 408);
    assert_eq!(poll_count(&polls), 1);
}

#[test]
fn sync_iteration_runs_failure_handlers_without_a_runtime() {
    let failure = Arc::new(FailurePort::default());
    let mut ports = Ports::default();
    ports.failure = Some(Arc::clone(&failure));
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(CREATED)]), None);
    futures_executor::block_on(stream.drive(Dispatch::Inline))
        .expect("the created event yields")
        .expect("an event is returned");
    let error = futures_executor::block_on(stream.drive(Dispatch::Inline))
        .expect_err("the stream ends early");
    connection_message(&error);
    assert_eq!(failure.classes().len(), 1);
}

#[test]
fn sync_iteration_logs_a_completed_response_inline() {
    let success = Arc::new(SuccessPort::default());
    let mut ports = Ports::default();
    ports.success = Some(Arc::clone(&success));
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.stream(prefetched(vec![payload(COMPLETED)]), None);
    futures_executor::block_on(stream.drive(Dispatch::Inline))
        .expect("the terminal event yields")
        .expect("an event is returned");
    assert_eq!(success.async_calls().len(), 1);
    assert_eq!(success.sync_calls().len(), 1);
    assert!(success.dispatches().is_empty());
}

#[tokio::test]
async fn fake_response_iteration_slices_text_into_chunks() {
    let success = Arc::new(SuccessPort::default());
    let cache = Arc::new(RecordingCache::default());
    let mut ports = Ports::default();
    ports.success = Some(Arc::clone(&success));
    ports.cache = Some(Arc::clone(&cache));
    ports.cache_handler = Some(Arc::new(caching_handler()));
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.synthetic(None, true);
    let mut types = Vec::new();
    let mut deltas = Vec::new();
    while let Some(event) = stream
        .advance(Dispatch::Spawned)
        .await
        .expect("the synthetic event yields")
    {
        if event_type(&event) == "response.output_text.delta" {
            deltas.push(delta(&event));
        }
        types.push(event_type(&event));
    }
    assert_eq!(
        deltas,
        vec!["abcde".to_owned(), "fghij".to_owned(), "k".to_owned()]
    );
    assert_eq!(types.last().map(String::as_str), Some("response.completed"));
    await_pending_logging(&mut stream.callback).await;
    let dispatches = success.dispatches();
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].cache_hit, None);
    assert_eq!(cache.writes().len(), 1);
}

#[test]
fn cached_response_iteration_marks_cache_hit_and_skips_persistence() {
    let success = Arc::new(SuccessPort::default());
    let cache = Arc::new(RecordingCache::default());
    let mut ports = Ports::default();
    ports.success = Some(Arc::clone(&success));
    ports.cache = Some(Arc::clone(&cache));
    ports.cache_handler = Some(Arc::new(caching_handler()));
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.synthetic(Some(true), false);
    let mut deltas = Vec::new();
    loop {
        let event = futures_executor::block_on(stream.advance(Dispatch::Inline))
            .expect("the synthetic event yields");
        let Some(event) = event else { break };
        if event_type(&event) == "response.output_text.delta" {
            deltas.push(delta(&event));
        }
    }
    assert_eq!(
        deltas,
        vec!["abcde".to_owned(), "fghij".to_owned(), "k".to_owned()]
    );
    assert_eq!(success.async_calls().len(), 1);
    assert_eq!(success.sync_calls().len(), 1);
    assert_eq!(success.async_calls()[0].cache_hit, Some(true));
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn cached_response_iteration_logs_only_when_completed_is_yielded() {
    let success = Arc::new(SuccessPort::default());
    let mut ports = Ports::default();
    ports.success = Some(Arc::clone(&success));
    let fixture = Fixture::new(ports, fixed_clock());
    let mut stream = fixture.synthetic(Some(true), false);
    let first = stream
        .advance(Dispatch::Spawned)
        .await
        .expect("the first event yields")
        .expect("an event is returned");
    assert_ne!(event_type(&first), "response.completed");
    await_pending_logging(&mut stream.callback).await;
    assert!(success.dispatches().is_empty());
    assert!(success.async_calls().is_empty());
    assert!(success.sync_calls().is_empty());
}

#[test]
fn separate_synthetic_streams_keep_their_indices_and_callbacks_apart() {
    let success = Arc::new(SuccessPort::default());
    let mut ports = Ports::default();
    ports.success = Some(Arc::clone(&success));
    let fixture = Fixture::new(ports, fixed_clock());
    let mut first = fixture.synthetic(Some(true), false);
    let mut second = fixture.synthetic(Some(true), false);
    let first_event = futures_executor::block_on(first.advance(Dispatch::Inline))
        .expect("the first stream yields")
        .expect("an event is returned");
    let second_event = futures_executor::block_on(second.advance(Dispatch::Inline))
        .expect("the second stream yields")
        .expect("an event is returned");
    assert_eq!(event_type(&first_event), event_type(&second_event));
    while futures_executor::block_on(first.advance(Dispatch::Inline))
        .expect("the first stream yields")
        .is_some()
    {}
    while futures_executor::block_on(second.advance(Dispatch::Inline))
        .expect("the second stream yields")
        .is_some()
    {}
    futures_executor::block_on(await_pending_logging(&mut first.callback));
    futures_executor::block_on(await_pending_logging(&mut second.callback));
    assert_eq!(success.async_calls().len(), 2);
    assert_eq!(success.sync_calls().len(), 2);
}

#[tokio::test]
async fn a_stream_without_a_provider_names_provider_in_the_early_end_message() {
    let fixture = Fixture::new(Ports::default(), fixed_clock());
    let mut stream = LiveStream::new(
        StreamEngine {
            transform: Some(&fixture.transform),
            tokenization: Some(&fixture.tokenization),
            calculator: Some(&fixture.calculator),
            hooks: &fixture.hooks,
            logging: &fixture.logging,
            details: IndexMap::new(),
            limits: &fixture.limits,
        },
        BaseStreamStateInput {
            model: "gpt-4o",
            custom_llm_provider: None,
            litellm_metadata: None,
            request_data: None,
            call_type: Some("responses"),
            logging: LoggingSnapshot::read(&fixture.logging),
            headers: &fixture.headers,
            api_base: &fixture.api_base,
            times: StreamTimes {
                now: timestamp(START),
                created_timestamp: CREATED_AT,
            },
            max_streaming_duration_seconds: None,
        },
        prefetched(vec![payload(CREATED)]),
    );
    let error = stream.next_event().await.expect_err("the stream fails");
    assert_eq!(
        fallback(&error).context.message,
        "litellm.MidStreamFallbackError: litellm.APIConnectionError: provider closed the responses stream before any terminal event (response.completed, response.incomplete or response.failed)"
    );
}
