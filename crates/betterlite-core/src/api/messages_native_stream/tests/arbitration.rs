#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_models::decisions::Json;
use futures_util::poll;

use super::super::queue::QueueItem;
use super::super::registry::StreamRegistry;
use super::super::{ClientRelay, RelayLimits, StreamFailureCall};
use super::support::{
    Frame, Harness, ProviderError, Recorder, arm_hook, frame, pump_of, queue_of, read,
};
use crate::api::messages_stream_logging::SseChunk;

fn limits(queue_maxsize: usize) -> RelayLimits {
    RelayLimits {
        queue_maxsize,
        max_detached_stream_drains: 100,
    }
}

fn busy() -> ProviderError {
    ProviderError {
        kind: "RateLimitError",
        message: "busy",
    }
}

#[tokio::test]
async fn upstream_exception_arbitration() {
    a_connected_client_gets_the_original_exception().await;
    a_queued_exception_fires_the_hook_once_after_detach().await;
    an_already_detached_exception_fires_the_hook_once().await;
    the_scan_matches_identity_not_text().await;
    a_missing_hook_is_harmless().await;
    a_refused_failure_enqueue_fires_the_hook().await;
}

async fn a_connected_client_gets_the_original_exception() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(8), Arc::clone(&registry));
    let hook = arm_hook(&harness);
    forward_two_chunks(&harness, &mut relay).await;
    let pump = pump_of(&harness);
    harness.script.fail(busy()).await;
    assert_eq!(
        read(&mut relay).await,
        Frame::Error("RateLimitError(busy)".to_owned())
    );
    drop(relay);
    pump.wait().await;
    harness.worker.flush().await;
    assert_eq!(hook.calls(), Vec::<String>::new());
    assert_eq!(harness.billed(), 0);
    assert_the_failure_record(&harness);
    assert_eq!(registry.sizes().live_pumps, 0);
}

async fn forward_two_chunks(harness: &Harness, relay: &mut ClientRelay<Recorder, ProviderError>) {
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(relay).await, Frame::Chunk("one".to_owned()));
    harness.script.release(SseChunk::Bytes(frame("two"))).await;
    assert_eq!(read(relay).await, Frame::Chunk("two".to_owned()));
}

fn assert_the_failure_record(harness: &Harness) {
    let failures = harness.failures();
    assert_eq!(failures.len(), 1);
    assert_the_failure_identity(&failures[0]);
    assert_the_failure_payload(&failures[0]);
}

fn assert_the_failure_identity(call: &StreamFailureCall) {
    assert_eq!(call.failure.kind, "RateLimitError");
    assert_eq!(call.failure.message, "busy");
    assert_eq!(call.endpoint, EndpointType::Anthropic);
}

fn assert_the_failure_payload(call: &StreamFailureCall) {
    assert_eq!(call.raw_bytes, vec![frame("one"), frame("two")]);
    assert_eq!(
        call.request_body.get("model"),
        Some(&Json::Text("claude".to_owned()))
    );
}

// detach_first decides whether the failure is queued before the client goes away
async fn a_failure_after_one_chunk(detach_first: bool) {
    let (harness, mut relay) = Harness::start(limits(8), Arc::new(StreamRegistry::default()));
    let hook = arm_hook(&harness);
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    let pump = pump_of(&harness);
    if detach_first {
        drop(relay);
        harness.script.fail(busy()).await;
    } else {
        harness.script.fail(busy()).await;
        drop(relay);
    }
    pump.wait().await;
    assert_eq!(hook.calls(), vec!["RateLimitError(busy)".to_owned()]);
    assert_eq!(harness.failures().len(), 1);
    assert_eq!(harness.billed(), 0);
}

async fn a_queued_exception_fires_the_hook_once_after_detach() {
    a_failure_after_one_chunk(false).await;
}

async fn an_already_detached_exception_fires_the_hook_once() {
    a_failure_after_one_chunk(true).await;
}

async fn the_scan_matches_identity_not_text() {
    let queue = queue_of(8);
    let first = Arc::new(busy());
    let same_text = Arc::new(busy());
    assert!(queue.enqueue(QueueItem::Chunk(frame("one"))).await);
    assert!(queue.enqueue(QueueItem::Failed(Arc::clone(&first))).await);
    assert!(
        queue
            .enqueue(QueueItem::Failed(Arc::clone(&same_text)))
            .await
    );
    assert!(queue.take_all_contains(&first));
    let mut next = Box::pin(queue.next());
    assert!(poll!(&mut next).is_pending());
    assert!(queue.enqueue(QueueItem::Failed(same_text)).await);
    assert!(!queue.take_all_contains(&first));
}

async fn a_missing_hook_is_harmless() {
    let (harness, mut relay) = Harness::start(limits(8), Arc::new(StreamRegistry::default()));
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    let pump = pump_of(&harness);
    drop(relay);
    harness.script.fail(busy()).await;
    pump.wait().await;
    assert_eq!(harness.failures().len(), 1);
    assert_eq!(harness.billed(), 0);
}

// the queue stays full while the client is connected, so this enqueue only loses to the detach
async fn a_refused_failure_enqueue_fires_the_hook() {
    let (harness, relay) = Harness::start(limits(1), Arc::new(StreamRegistry::default()));
    let hook = arm_hook(&harness);
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    let pump = pump_of(&harness);
    harness.script.fail(busy()).await;
    drop(relay);
    pump.wait().await;
    assert_eq!(hook.calls(), vec!["RateLimitError(busy)".to_owned()]);
    assert_eq!(harness.billed(), 0);
}
