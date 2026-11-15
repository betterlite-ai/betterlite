#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::Poll;

use futures_util::poll;
use tokio::task::yield_now;

use super::super::queue::QueueItem;
use super::super::registry::StreamRegistry;
use super::super::{ClientRelay, RelayLimits};
use super::support::{
    Frame, Harness, ProviderError, Recorder, frame, pump_of, queue_of, read, settle,
};
use crate::api::messages_sse_events::render::incomplete_error_sse_event;
use crate::api::messages_stream_logging::SseChunk;

fn limits(queue_maxsize: usize) -> RelayLimits {
    RelayLimits {
        queue_maxsize,
        max_detached_stream_drains: 100,
    }
}

#[tokio::test]
async fn connected_backpressure() {
    two_queued_chunks_hold_the_third_pull().await;
    disconnect_resolves_a_blocked_enqueue().await;
    an_already_detached_enqueue_returns_false_at_once().await;
    a_completed_put_wins_over_detach().await;
    a_lost_put_leaves_nothing_queued().await;
}

async fn two_queued_chunks_hold_the_third_pull() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(2), Arc::clone(&registry));
    release_three_chunks(&harness).await;
    harness.script.try_release(SseChunk::Bytes(frame("four")));
    // a pump with room reaches the fourth command; this one stays parked on the full queue
    yield_now().await;
    assert_eq!(harness.script.pulls(), 3);
    drain_four_chunks(&harness, &mut relay).await;
    harness.script.finish().await;
    let incomplete = String::from_utf8(incomplete_error_sse_event()).expect("the event is UTF-8");
    assert_eq!(read(&mut relay).await, Frame::Chunk(incomplete));
    assert_eq!(read(&mut relay).await, Frame::End);
    assert_billing(&harness).await;
    settle(&harness, relay).await;
    assert_eq!(registry.sizes().live_pumps, 0);
}

async fn release_three_chunks(harness: &Harness) {
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    harness.script.release(SseChunk::Bytes(frame("two"))).await;
    harness
        .script
        .release(SseChunk::Bytes(frame("three")))
        .await;
}

async fn drain_four_chunks(harness: &Harness, relay: &mut ClientRelay<Recorder, ProviderError>) {
    assert_eq!(read(relay).await, Frame::Chunk("one".to_owned()));
    harness.script.await_delivery().await;
    assert_eq!(read(relay).await, Frame::Chunk("two".to_owned()));
    assert_eq!(read(relay).await, Frame::Chunk("three".to_owned()));
    assert_eq!(read(relay).await, Frame::Chunk("four".to_owned()));
}

async fn assert_billing(harness: &Harness) {
    harness.worker.flush().await;
    assert_eq!(harness.ran(), 1);
    assert_eq!(
        harness.routed_frames(),
        ["one", "two", "three", "four"].map(frame).to_vec()
    );
}

async fn disconnect_resolves_a_blocked_enqueue() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(1), Arc::clone(&registry));
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    harness.script.release(SseChunk::Bytes(frame("two"))).await;
    harness
        .script
        .release(SseChunk::Bytes(frame("three")))
        .await;
    let pump = pump_of(&harness);
    drop(relay);
    harness.script.finish().await;
    pump.wait().await;
    harness.worker.flush().await;
    assert_eq!(
        harness.routed_frames(),
        ["one", "two", "three"].map(frame).to_vec()
    );
    assert_eq!(harness.ran(), 1);
    assert!(!harness.closed.load(Ordering::SeqCst));
    assert_eq!(registry.sizes().live_pumps, 0);
}

async fn an_already_detached_enqueue_returns_false_at_once() {
    let queue = queue_of(1);
    queue.detach();
    let mut enqueue = Box::pin(queue.enqueue(QueueItem::Chunk(frame("late"))));
    assert_eq!(poll!(&mut enqueue), Poll::Ready(false));
}

async fn a_completed_put_wins_over_detach() {
    let queue = queue_of(1);
    assert!(queue.enqueue(QueueItem::Chunk(frame("first"))).await);
    let mut blocked = Box::pin(queue.enqueue(QueueItem::Chunk(frame("second"))));
    assert!(poll!(&mut blocked).is_pending());
    assert!(matches!(
        queue.next().await,
        Some(QueueItem::Chunk(bytes)) if bytes == frame("first")
    ));
    queue.detach();
    assert_eq!(poll!(&mut blocked), Poll::Ready(true));
    assert!(matches!(
        queue.next().await,
        Some(QueueItem::Chunk(bytes)) if bytes == frame("second")
    ));
}

async fn a_lost_put_leaves_nothing_queued() {
    let queue = queue_of(1);
    assert!(queue.enqueue(QueueItem::Chunk(frame("first"))).await);
    let mut blocked = Box::pin(queue.enqueue(QueueItem::Chunk(frame("second"))));
    assert!(poll!(&mut blocked).is_pending());
    queue.detach();
    assert_eq!(poll!(&mut blocked), Poll::Ready(false));
    assert!(matches!(queue.next().await, Some(QueueItem::Chunk(_))));
    // the cancelled put never landed: only the first chunk was in the queue
    let mut next = Box::pin(queue.next());
    assert!(poll!(&mut next).is_pending());
}
