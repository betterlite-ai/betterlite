#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio_util::task::TaskTracker;

use super::super::RelayLimits;
use super::super::registry::{RegistrySizes, StreamRegistry};
use super::support::{Frame, Harness, capture_warnings, frame, pump_of, read};
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
    let (a, pump_a) = a_held_slot_caps_the_second_drain(&registry).await;
    a_cap_hit_stream_bills_and_aborts(&registry).await;
    the_holder_drains_completely_and_releases(&registry, &a, &pump_a).await;
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
    let (a, mut relay_a) = Harness::start(limits(1), Arc::clone(registry));
    a.script.release(SseChunk::Bytes(frame("a1"))).await;
    assert_eq!(read(&mut relay_a).await, Frame::Chunk("a1".to_owned()));
    let pump_a = pump_of(&relay_a);
    drop(relay_a);
    a.script.release(SseChunk::Bytes(frame("a2"))).await;
    assert_eq!(registry.sizes().detached_drains, 1);
    let (d, mut relay_d) = Harness::start(limits(1), Arc::clone(registry));
    d.script.release(SseChunk::Bytes(frame("d1"))).await;
    assert_eq!(read(&mut relay_d).await, Frame::Chunk("d1".to_owned()));
    let pump_d = pump_of(&relay_d);
    drop(relay_d);
    d.script.finish().await;
    pump_d.wait().await;
    d.worker.flush().await;
    // a stream that ends right after the disconnect bills without claiming a slot
    assert_eq!(d.routed_frames(), vec![frame("d1")]);
    assert_eq!(registry.sizes().detached_drains, 1);
    (a, pump_a)
}

async fn a_cap_hit_stream_bills_and_aborts(registry: &Arc<StreamRegistry>) {
    let (b, mut relay_b) = Harness::start(limits(1), Arc::clone(registry));
    b.script.release(SseChunk::Bytes(frame("b1"))).await;
    assert_eq!(read(&mut relay_b).await, Frame::Chunk("b1".to_owned()));
    let pump = pump_of(&relay_b);
    drop(relay_b);
    b.script.release(SseChunk::Bytes(frame("b2"))).await;
    pump.wait().await;
    b.worker.flush().await;
    assert!(b.closed.load(Ordering::SeqCst));
    assert_eq!(b.routed_frames(), vec![frame("b1"), frame("b2")]);
    assert_eq!(b.ran(), 1);
    assert_eq!(registry.sizes().live_pumps, 1);
}

async fn the_holder_drains_completely_and_releases(
    registry: &Arc<StreamRegistry>,
    a: &Harness,
    pump: &Arc<TaskTracker>,
) {
    a.script.release(SseChunk::Bytes(frame("a3"))).await;
    a.script.finish().await;
    pump.wait().await;
    a.worker.flush().await;
    assert_eq!(
        a.routed_frames(),
        vec![frame("a1"), frame("a2"), frame("a3")]
    );
    assert_eq!(a.ran(), 1);
    assert_eq!(
        registry.sizes(),
        RegistrySizes {
            live_pumps: 0,
            detached_drains: 0,
        }
    );
}

async fn a_later_stream_claims_the_released_slot(registry: &Arc<StreamRegistry>) {
    let (c, mut relay_c) = Harness::start(limits(1), Arc::clone(registry));
    c.script.release(SseChunk::Bytes(frame("c1"))).await;
    assert_eq!(read(&mut relay_c).await, Frame::Chunk("c1".to_owned()));
    let pump = pump_of(&relay_c);
    drop(relay_c);
    c.script.release(SseChunk::Bytes(frame("c2"))).await;
    assert_eq!(registry.sizes().detached_drains, 1);
    c.script.finish().await;
    pump.wait().await;
    c.worker.flush().await;
    assert_eq!(c.routed_frames(), vec![frame("c1"), frame("c2")]);
    assert_eq!(c.ran(), 1);
    assert_eq!(registry.sizes().detached_drains, 0);
}

async fn a_zero_cap_bills_every_disconnect(registry: &Arc<StreamRegistry>) {
    let (z, mut relay_z) = Harness::start(limits(0), Arc::clone(registry));
    z.script.release(SseChunk::Bytes(frame("z1"))).await;
    assert_eq!(read(&mut relay_z).await, Frame::Chunk("z1".to_owned()));
    let pump = pump_of(&relay_z);
    drop(relay_z);
    z.script.release(SseChunk::Bytes(frame("z2"))).await;
    pump.wait().await;
    z.worker.flush().await;
    assert!(z.closed.load(Ordering::SeqCst));
    assert_eq!(z.routed_frames(), vec![frame("z1"), frame("z2")]);
    assert_eq!(z.ran(), 1);
    assert_eq!(registry.sizes().detached_drains, 0);
}
