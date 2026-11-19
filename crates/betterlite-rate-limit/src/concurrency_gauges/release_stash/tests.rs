#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, PlannedScript, RecordingCache, RecordingReleaseBatch, acquisition,
    empty_stash, gauges_with, int_counter, int_values, key_list, registry, stash_with,
};
use crate::concurrency_gauges::{ConcurrencyGauges, PARALLEL_SLOT_TTL};
use crate::rate_limit_window_store::CounterScript;
use crate::token_adjustments::store::PostCallScriptBatch;

const NOW: f64 = 1_700_000_000.0;

fn stash_gauges(
    cache: Arc<RecordingCache>,
    batch: Option<Arc<RecordingReleaseBatch>>,
) -> (ConcurrencyGauges, Arc<PlannedScript>) {
    let mut gauges = gauges_with(cache, FixedClock::at(NOW));
    let handle = PlannedScript::new("release", vec![int_values(&[1])]);
    let script: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(&handle);
    gauges.release_script = Some(script);
    if let Some(recording) = batch {
        let shared: Arc<dyn PostCallScriptBatch> = recording;
        gauges.release_batch = Some(shared);
    }
    (gauges, handle)
}

#[tokio::test]
async fn immediate_release_when_not_in_a_logging_callback() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let batch = RecordingReleaseBatch::new();
    let (gauges, script) = stash_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let stash = stash_with(acquisition("slot-a", &["g1"]));
    gauges
        .release_stashed_parallel_slot(Some(&stash), false)
        .await;
    assert_eq!(script.calls().len(), 1);
    assert!(batch.registrations().is_empty());
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: int_counter(1),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
    assert!(
        stash
            .state
            .lock()
            .expect("the stash lock")
            .parallel_slot
            .is_none()
    );
}

#[tokio::test]
async fn deferred_release_registers_with_the_batch() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let batch = RecordingReleaseBatch::new();
    let (gauges, script) = stash_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let stash = stash_with(acquisition("slot-a", &["g1"]));
    gauges
        .release_stashed_parallel_slot(Some(&stash), true)
        .await;
    assert!(script.calls().is_empty());
    let registrations = batch.registrations();
    assert_eq!(registrations.len(), 1);
    let registration = registrations.first().expect("one registration");
    assert_eq!(registration.keys, key_list(&["g1"]));
    assert_eq!(
        registration.args,
        vec![RedisScalar::Text("slot-a".to_owned())]
    );
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-b", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
    assert!(
        stash
            .state
            .lock()
            .expect("the stash lock")
            .parallel_slot
            .is_none()
    );
}

#[tokio::test]
async fn already_released_slot_returns_without_a_write() {
    let cache = RecordingCache::new();
    let batch = RecordingReleaseBatch::new();
    let (gauges, script) = stash_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let stash = empty_stash();
    gauges
        .release_stashed_parallel_slot(Some(&stash), true)
        .await;
    gauges
        .release_stashed_parallel_slot(Some(&stash), false)
        .await;
    assert!(script.calls().is_empty());
    assert!(batch.registrations().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn deferred_release_without_a_batch_falls_back_to_the_direct_release() {
    let cache = RecordingCache::new();
    let (gauges, script) = stash_gauges(Arc::clone(&cache), None);
    let stash = stash_with(acquisition("slot-a", &["g1"]));
    gauges
        .release_stashed_parallel_slot(Some(&stash), true)
        .await;
    assert_eq!(script.calls().len(), 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: int_counter(1),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_missing_stash_releases_nothing() {
    let cache = RecordingCache::new();
    let (gauges, script) = stash_gauges(Arc::clone(&cache), None);
    gauges.release_stashed_parallel_slot(None, true).await;
    assert!(script.calls().is_empty());
    assert!(cache.writes().is_empty());
}
