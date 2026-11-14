#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio_util::task::TaskTracker;

use super::super::RelayLimits;
use super::super::registry::{RegistrySizes, StreamRegistry};
use super::support::{Frame, Harness, capture_warnings, frame, pump_of, read, wait_for_drains};
use crate::api::messages_stream_logging::SseChunk;

const CAP_WARNING: &str = "async_sse_wrapper: detached-drain cap (1) reached; billing 2 partial chunks and aborting the upstream stream to stop provider billing";

fn limits(max_detached_stream_drains: usize) -> RelayLimits {
    RelayLimits {
        queue_maxsize: 8,
        max_detached_stream_drains,
    }
}

#[tokio::test]
async fn global_detached_drain_cap() {
    let registry = Arc::new(StreamRegistry::default());
    let (captured, _guard) = capture_warnings();
    let (holder, pump_holder) = a_held_slot_caps_the_second_drain(&registry).await;
    a_cap_hit_stream_bills_and_aborts(&registry).await;
    the_holder_drains_completely_and_releases(&registry, &holder, &pump_holder).await;
    a_later_stream_claims_the_released_slot(&registry).await;
    a_zero_cap_bills_every_disconnect(&registry).await;
    let text = captured.text();
    assert_eq!(text.matches(CAP_WARNING).count(), 1, "{text}");
    assert_eq!(text.matches("detached-drain cap (0) reached").count(), 1);
    assert_eq!(
        registry.sizes(),
        RegistrySizes {
            live_pumps: 0,
            detached_drains: 0,
        }
    );
}

async fn a_held_slot_caps_the_second_drain(
    registry: &Arc<StreamRegistry>,
) -> (Harness, Arc<TaskTracker>) {
    let (holder, mut relay_holder) = Harness::start(limits(1), Arc::clone(registry));
    holder.script.release(SseChunk::Bytes(frame("a1"))).await;
    assert_eq!(read(&mut relay_holder).await, Frame::Chunk("a1".to_owned()));
    let pump_holder = pump_of(&holder);
    drop(relay_holder);
    holder.script.release(SseChunk::Bytes(frame("a2"))).await;
    wait_for_drains(registry, 1).await;
    let (drainer, mut relay_drainer) = Harness::start(limits(1), Arc::clone(registry));
    drainer.script.release(SseChunk::Bytes(frame("d1"))).await;
    assert_eq!(
        read(&mut relay_drainer).await,
        Frame::Chunk("d1".to_owned())
    );
    let pump_drainer = pump_of(&drainer);
    drop(relay_drainer);
    drainer.script.finish().await;
    pump_drainer.wait().await;
    drainer.worker.flush().await;
    // a stream that ends right after the disconnect bills without claiming a slot
    assert_eq!(drainer.routed_frames(), vec![frame("d1")]);
    assert_eq!(registry.sizes().detached_drains, 1);
    (holder, pump_holder)
}

// A disconnect whose next chunk cannot claim a drain slot bills and aborts upstream
async fn a_disconnect_beyond_the_cap(registry: &Arc<StreamRegistry>, cap: usize) {
    let (harness, mut relay) = Harness::start(limits(cap), Arc::clone(registry));
    harness.script.release(SseChunk::Bytes(frame("one"))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk("one".to_owned()));
    let pump = pump_of(&harness);
    drop(relay);
    harness.script.release(SseChunk::Bytes(frame("two"))).await;
    pump.wait().await;
    harness.worker.flush().await;
    assert!(harness.closed.load(Ordering::SeqCst));
    assert_eq!(harness.routed_frames(), vec![frame("one"), frame("two")]);
    assert_eq!(harness.ran(), 1);
}

async fn a_cap_hit_stream_bills_and_aborts(registry: &Arc<StreamRegistry>) {
    a_disconnect_beyond_the_cap(registry, 1).await;
    assert_eq!(registry.sizes().live_pumps, 1);
}

async fn the_holder_drains_completely_and_releases(
    registry: &Arc<StreamRegistry>,
    holder: &Harness,
    pump: &Arc<TaskTracker>,
) {
    holder.script.release(SseChunk::Bytes(frame("a3"))).await;
    holder.script.finish().await;
    pump.wait().await;
    holder.worker.flush().await;
    assert_eq!(
        holder.routed_frames(),
        vec![frame("a1"), frame("a2"), frame("a3")]
    );
    assert_eq!(holder.ran(), 1);
    assert_eq!(
        registry.sizes(),
        RegistrySizes {
            live_pumps: 0,
            detached_drains: 0,
        }
    );
}

async fn a_later_stream_claims_the_released_slot(registry: &Arc<StreamRegistry>) {
    let (claimer, mut relay_claimer) = Harness::start(limits(1), Arc::clone(registry));
    claimer.script.release(SseChunk::Bytes(frame("c1"))).await;
    assert_eq!(
        read(&mut relay_claimer).await,
        Frame::Chunk("c1".to_owned())
    );
    let pump = pump_of(&claimer);
    drop(relay_claimer);
    claimer.script.release(SseChunk::Bytes(frame("c2"))).await;
    wait_for_drains(registry, 1).await;
    claimer.script.finish().await;
    pump.wait().await;
    claimer.worker.flush().await;
    assert_eq!(claimer.routed_frames(), vec![frame("c1"), frame("c2")]);
    assert_eq!(claimer.ran(), 1);
    assert_eq!(registry.sizes().detached_drains, 0);
}

async fn a_zero_cap_bills_every_disconnect(registry: &Arc<StreamRegistry>) {
    a_disconnect_beyond_the_cap(registry, 0).await;
    assert_eq!(registry.sizes().detached_drains, 0);
}
