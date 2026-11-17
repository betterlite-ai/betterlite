#![cfg(test)]

use std::sync::PoisonError;

use crate::passthrough_stream::tests::{
    Fixture, ReplyFixture, SyncFixture, capture, chunk_steps, inline_executor, script,
};

#[tokio::test]
async fn async_flush_runs_once_after_the_stream_ends() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one"]))));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert_eq!(fixture.logging.flushed().len(), 1);
    super::start_async_flush(&mut fixture.stream);
    fixture.settled().await;
    assert_eq!(fixture.logging.flushed().len(), 1);
    assert!(fixture.stream.flush_scheduled);
}

#[tokio::test]
async fn async_flush_is_skipped_without_chunks() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert!(fixture.logging.flushed().is_empty());
    assert!(!fixture.stream.flush_scheduled);
    super::start_async_flush(&mut fixture.stream);
    assert!(!fixture.stream.flush_scheduled);
    assert!(fixture.tracker.is_empty());
}

#[tokio::test]
async fn async_flush_is_skipped_after_a_collector_failure() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one", "two"]))));
    fixture
        .sink
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    assert_eq!(
        fixture.stream.next_chunk().await.expect("second chunk"),
        Some(b"two".to_vec())
    );
    assert_eq!(fixture.stream.next_chunk().await.expect("end"), None);
    fixture.settled().await;
    assert!(fixture.logging.flushed().is_empty());
    assert!(!fixture.stream.flush_scheduled);
}

#[test]
fn async_flush_outside_a_runtime_is_reported() {
    let (captured, _guard) = capture();
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(Vec::new())));
    fixture.stream.spend.add(b"one");
    super::start_async_flush(&mut fixture.stream);
    assert!(fixture.stream.flush_scheduled);
    assert!(fixture.tracker.is_empty());
    assert!(fixture.logging.flushed().is_empty());
    let messages = captured.messages();
    assert!(messages.iter().any(|message| message.contains(
        "Failed to schedule passthrough spend-tracking flush; 1 collected chunks dropped"
    )));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("no async runtime is running"))
    );
}

#[test]
fn sync_flush_runs_once_via_the_executor() {
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(Vec::new())), inline_executor());
    fixture.stream.spend.add(b"one");
    super::start_sync_flush(&mut fixture.stream);
    super::start_sync_flush(&mut fixture.stream);
    assert_eq!(fixture.executor.scheduled(), 1);
    assert!(fixture.stream.flush_scheduled);
    fixture.executor.run_jobs();
    let flushes = fixture.logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
}

#[test]
fn sync_flush_is_skipped_without_chunks() {
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(Vec::new())), inline_executor());
    super::start_sync_flush(&mut fixture.stream);
    assert_eq!(fixture.executor.scheduled(), 0);
    assert!(!fixture.stream.flush_scheduled);
    assert!(fixture.logging.flushed().is_empty());
}

#[test]
fn sync_flush_is_skipped_after_a_collector_failure() {
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(Vec::new())), inline_executor());
    fixture
        .sink
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    fixture.stream.spend.add(b"one");
    super::start_sync_flush(&mut fixture.stream);
    assert_eq!(fixture.executor.scheduled(), 0);
    assert!(!fixture.stream.flush_scheduled);
    assert!(fixture.logging.flushed().is_empty());
}

#[test]
fn sync_flush_reports_a_rejected_job() {
    let mut executor = inline_executor();
    executor.reject = true;
    let (captured, _guard) = capture();
    let mut fixture = SyncFixture::new(ReplyFixture::ok(script(Vec::new())), executor);
    fixture.stream.spend.add(b"one");
    super::start_sync_flush(&mut fixture.stream);
    assert!(fixture.stream.flush_scheduled);
    assert!(fixture.logging.flushed().is_empty());
    let messages = captured.messages();
    assert!(messages.iter().any(|message| message.contains(
        "Failed to schedule passthrough spend-tracking flush; 1 collected chunks dropped"
    )));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("the executor is shut down"))
    );
}
