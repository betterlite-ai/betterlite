#![cfg(test)]

use std::sync::PoisonError;

use betterlite_http::error::HttpError;
use http::HeaderValue;

use crate::passthrough_stream::tests::{
    ReplyFixture, Step, SyncFixture, capture, chunk_steps, inline_executor, script,
};

#[test]
fn new_exposes_status_and_headers_immediately() {
    let fixture = SyncFixture::new(
        ReplyFixture::ok(script(Vec::new())).with_status(201),
        inline_executor(),
    );
    assert_eq!(fixture.stream.status_code, 201);
    assert_eq!(
        fixture.stream.headers.get("x-upstream"),
        Some(&HeaderValue::from_static("yes"))
    );
    assert!(!fixture.stream.flush_scheduled);
}

#[test]
fn iteration_passes_chunks_and_feeds_the_sink() {
    let mut fixture = SyncFixture::new(
        ReplyFixture::ok(script(chunk_steps(&["one", "two"]))),
        inline_executor(),
    );
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(
        fixture.stream.next_chunk().expect("second chunk"),
        Some(b"two".to_vec())
    );
    let chunks = fixture
        .sink
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .chunks
        .clone();
    assert_eq!(chunks, vec![b"one".to_vec(), b"two".to_vec()]);
}

#[test]
fn end_of_stream_submits_the_flush_once() {
    let mut fixture = SyncFixture::new(
        ReplyFixture::ok(script(chunk_steps(&["one"]))),
        inline_executor(),
    );
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().expect("end"), None);
    assert_eq!(fixture.executor.scheduled(), 1);
    assert!(fixture.stream.flush_scheduled);
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
    fixture.executor.run_jobs();
    let flushes = fixture.logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
    fixture.stream.close();
    assert_eq!(fixture.executor.scheduled(), 0);
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        2
    );
}

#[test]
fn empty_stream_submits_nothing() {
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(Vec::new())), inline_executor());
    assert_eq!(fixture.stream.next_chunk().expect("end"), None);
    assert_eq!(fixture.executor.scheduled(), 0);
    assert!(!fixture.stream.flush_scheduled);
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[test]
fn read_error_submits_the_flush_and_closes() {
    let steps = vec![Step::Chunk(b"one".to_vec()), Step::Fail];
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(steps)), inline_executor());
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    let error = fixture.stream.next_chunk().expect_err("the source fails");
    assert!(matches!(error, HttpError::Timeout));
    assert_eq!(fixture.executor.scheduled(), 1);
    fixture.executor.run_jobs();
    assert_eq!(fixture.logging.flushed()[0].logged, "one");
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[test]
fn iterator_next_yields_chunks_until_the_end() {
    let mut fixture = SyncFixture::new(
        ReplyFixture::ok(script(chunk_steps(&["one", "two"]))),
        inline_executor(),
    );
    let mut seen = Vec::new();
    while let Some(item) = fixture.stream.next() {
        seen.push(item.expect("the scripted chunk arrives"));
    }
    assert_eq!(seen, vec![b"one".to_vec(), b"two".to_vec()]);
    assert_eq!(fixture.executor.scheduled(), 1);
}

#[test]
fn close_without_chunks_does_not_submit() {
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(Vec::new())), inline_executor());
    fixture.stream.close();
    fixture.stream.close();
    assert_eq!(fixture.executor.scheduled(), 0);
    assert!(!fixture.stream.flush_scheduled);
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        2
    );
}

#[test]
fn close_after_chunks_submits_the_flush_once() {
    let mut fixture = SyncFixture::new(
        ReplyFixture::ok(script(chunk_steps(&["one"]))),
        inline_executor(),
    );
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    fixture.stream.close();
    fixture.stream.close();
    assert_eq!(fixture.executor.scheduled(), 1);
    fixture.executor.run_jobs();
    assert_eq!(fixture.logging.flushed().len(), 1);
}

#[test]
fn a_rejected_job_is_logged_with_the_chunk_count() {
    let mut executor = inline_executor();
    executor.reject = true;
    let (captured, _guard) = capture();
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(chunk_steps(&["one"]))), executor);
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().expect("end"), None);
    assert!(fixture.stream.flush_scheduled);
    let messages = captured.messages();
    assert!(messages.iter().any(|message| message.contains(
        "Failed to schedule passthrough spend-tracking flush; 1 collected chunks dropped"
    )));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("the executor is shut down"))
    );
    assert!(fixture.logging.flushed().is_empty());
}

#[test]
fn close_swallows_a_close_error() {
    let (captured, _guard) = capture();
    let mut fixture = SyncFixture::new(
        ReplyFixture::ok(script(chunk_steps(&["one"]))).failing_close(),
        inline_executor(),
    );
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    fixture.stream.close();
    assert_eq!(fixture.executor.scheduled(), 1);
    assert!(
        captured
            .messages()
            .iter()
            .any(|message| message.contains("the passthrough response could not be closed"))
    );
}

#[test]
fn a_failing_collector_does_not_break_the_sync_relay() {
    let mut fixture = SyncFixture::new(
        ReplyFixture::ok(script(chunk_steps(&["one", "two"]))),
        inline_executor(),
    );
    fixture
        .sink
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    assert_eq!(
        fixture.stream.next_chunk().expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(
        fixture.stream.next_chunk().expect("second chunk"),
        Some(b"two".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().expect("end"), None);
    assert_eq!(fixture.executor.scheduled(), 0);
    assert_eq!(
        fixture
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls,
        1
    );
}
