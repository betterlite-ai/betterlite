#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::passthrough_stream::error::PassthroughStreamError;
use crate::passthrough_stream::tests::{
    Fixture, ReplyFixture, SourceLog, Step, capture, chunk_steps, flagged_pending, script,
};

#[tokio::test]
async fn aclose_flushes_the_collected_chunks_and_closes() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one"]))));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    fixture.stream.aclose().await;
    fixture.settled().await;
    assert!(fixture.stream.flush_scheduled);
    let flushes = fixture.logging.flushed();
    assert_eq!(flushes.len(), 1);
    assert_eq!(flushes[0].logged, "one");
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[tokio::test]
async fn aclose_twice_flushes_once_and_closes_twice() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(chunk_steps(&["one"]))));
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    fixture.stream.aclose().await;
    fixture.stream.aclose().await;
    fixture.settled().await;
    assert_eq!(fixture.logging.flushed().len(), 1);
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        2
    );
}

#[tokio::test]
async fn aclose_before_init_does_not_send_or_flush() {
    let awaited = Arc::new(AtomicBool::new(false));
    let pending = flagged_pending(
        ReplyFixture::ok(script(chunk_steps(&["one"]))),
        Arc::clone(&awaited),
    );
    let mut fixture = Fixture::from_pending(pending, Arc::new(Mutex::new(SourceLog::default())));
    fixture.stream.aclose().await;
    fixture.settled().await;
    assert!(!awaited.load(Ordering::SeqCst));
    assert!(!fixture.stream.initialized);
    assert!(!fixture.stream.flush_scheduled);
    assert!(fixture.logging.flushed().is_empty());
}

#[tokio::test]
async fn aclose_after_init_without_chunks_does_not_flush() {
    let mut fixture = Fixture::ready(ReplyFixture::ok(script(Vec::new()))).await;
    fixture.stream.aclose().await;
    fixture.settled().await;
    assert!(!fixture.stream.flush_scheduled);
    assert!(fixture.logging.flushed().is_empty());
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        1
    );
}

#[tokio::test]
async fn aclose_swallows_a_close_error() {
    let (captured, _guard) = capture();
    let mut fixture =
        Fixture::ready(ReplyFixture::ok(script(chunk_steps(&["one"]))).failing_close()).await;
    assert_eq!(
        fixture.stream.next_chunk().await.expect("first chunk"),
        Some(b"one".to_vec())
    );
    fixture.stream.aclose().await;
    fixture.settled().await;
    assert_eq!(fixture.logging.flushed().len(), 1);
    assert!(
        captured
            .messages()
            .iter()
            .any(|message| message.contains("could not be closed after the stream ended"))
    );
}

#[tokio::test]
async fn aclose_after_a_stream_error_flushes_nothing() {
    let mut fixture = Fixture::awaiting(ReplyFixture::ok(script(vec![Step::Fail])));
    let error = fixture
        .stream
        .next_chunk()
        .await
        .expect_err("the source fails");
    assert!(matches!(error, PassthroughStreamError::StreamRead(_)));
    fixture.stream.aclose().await;
    fixture.settled().await;
    assert!(fixture.logging.flushed().is_empty());
    assert_eq!(
        fixture
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed,
        2
    );
}
