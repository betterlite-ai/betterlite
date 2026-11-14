#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::super::RelayLimits;
use super::super::registry::StreamRegistry;
use super::support::{Frame, Harness, arm_deferred, frame, pump_of, read, settle};
use crate::api::messages_stream_logging::SseChunk;

fn limits(queue_maxsize: usize) -> RelayLimits {
    RelayLimits {
        queue_maxsize,
        max_detached_stream_drains: 100,
    }
}

const STOP: &str = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

#[tokio::test]
async fn teardown_dispatches_parked_logging() {
    a_disconnect_dispatches_the_parked_callback_once().await;
    a_sentinel_that_loses_to_detach_dispatches_in_the_pump().await;
    a_detached_completion_bills_with_teardown().await;
    a_consumed_sentinel_leaves_the_parked_callback_to_the_proxy().await;
}

async fn a_disconnect_dispatches_the_parked_callback_once() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(3), Arc::clone(&registry));
    let deferred = arm_deferred(&harness);
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    harness.script.release(SseChunk::Bytes(frame(STOP))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(STOP.to_owned()));
    let pump = pump_of(&relay);
    harness.script.finish().await;
    pump.wait().await;
    assert_eq!(harness.ran(), 0);
    assert!(harness.parked());
    drop(relay);
    harness.worker.flush().await;
    assert_eq!(deferred.calls(), vec![1]);
    assert_eq!(harness.ran(), 1);
    assert_eq!(harness.routed_frames(), vec![frame("one"), frame(STOP)]);
    assert_eq!(registry.sizes().live_pumps, 0);
}

async fn a_sentinel_that_loses_to_detach_dispatches_in_the_pump() {
    let (harness, mut relay) = Harness::start(limits(2), Arc::new(StreamRegistry::default()));
    let deferred = arm_deferred(&harness);
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    harness.script.release(SseChunk::Bytes(frame("two"))).await;
    let pump = pump_of(&relay);
    harness.script.finish().await;
    while !harness.parked() {
        tokio::task::yield_now().await;
    }
    drop(relay);
    pump.wait().await;
    harness.worker.flush().await;
    assert_eq!(deferred.calls(), vec![1]);
    assert_eq!(harness.ran(), 1);
    assert_eq!(harness.routed_frames(), vec![frame("one"), frame("two")]);
}

async fn a_detached_completion_bills_with_teardown() {
    let (harness, mut relay) = Harness::start(limits(2), Arc::new(StreamRegistry::default()));
    let deferred = arm_deferred(&harness);
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    let pump = pump_of(&relay);
    drop(relay);
    harness.script.finish().await;
    pump.wait().await;
    harness.worker.flush().await;
    // teardown billing runs even with a deferred callback armed, so the callback never sees it
    assert_eq!(harness.ran(), 1);
    assert_eq!(deferred.calls(), Vec::<usize>::new());
    assert!(!harness.parked());
    assert_eq!(harness.routed_frames(), vec![frame("one")]);
    assert!(!harness.closed.load(Ordering::SeqCst));
}

async fn a_consumed_sentinel_leaves_the_parked_callback_to_the_proxy() {
    let (harness, mut relay) = Harness::start(limits(2), Arc::new(StreamRegistry::default()));
    let deferred = arm_deferred(&harness);
    harness.script.release(SseChunk::Bytes(frame(STOP))).await;
    harness.script.finish().await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(STOP.to_owned()));
    assert_eq!(read(&mut relay).await, Frame::End);
    assert!(relay.reached_end);
    settle(relay).await;
    harness.worker.flush().await;
    assert_eq!(harness.ran(), 0);
    assert_eq!(deferred.calls(), Vec::<usize>::new());
    assert!(harness.parked());
}
