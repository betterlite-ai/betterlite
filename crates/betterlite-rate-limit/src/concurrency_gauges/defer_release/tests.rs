#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::batch::result::Settled;
use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, LogCapture, PlannedScript, RecordingCache, RecordingReleaseBatch,
    acquisition, batch_failure, gauges_with, int_values, key_list, registry,
};
use crate::concurrency_gauges::{ConcurrencyGauges, PARALLEL_SLOT_TTL};
use crate::limiter_records::ScriptReply;
use crate::rate_limit_window_store::CounterScript;
use crate::token_adjustments::store::PostCallScriptBatch;

use super::defer_slot_release;

const NOW: f64 = 1_700_000_000.0;

fn deferring_gauges(
    cache: Arc<RecordingCache>,
    batch: Option<Arc<RecordingReleaseBatch>>,
) -> (ConcurrencyGauges, Arc<PlannedScript>) {
    let mut gauges = gauges_with(cache, FixedClock::at(NOW));
    let handle = PlannedScript::new("release", vec![int_values(&[0])]);
    let script: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(&handle);
    gauges.release_script = Some(script);
    if let Some(recording) = batch {
        let shared: Arc<dyn PostCallScriptBatch> = recording;
        gauges.release_batch = Some(shared);
    }
    (gauges, handle)
}

#[tokio::test]
async fn immediate_in_memory_release_before_the_batch() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let batch = RecordingReleaseBatch::new();
    let (gauges, script) = deferring_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let deferred = defer_slot_release(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert!(deferred);
    assert!(script.calls().is_empty());
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-b", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
    let registrations = batch.registrations();
    assert_eq!(registrations.len(), 1);
    let registration = registrations.first().expect("one registration");
    assert_eq!(registration.source, "release");
    assert_eq!(registration.keys, key_list(&["g1"]));
    assert_eq!(
        registration.args,
        vec![RedisScalar::Text("slot-a".to_owned())]
    );
}

#[tokio::test]
async fn the_batch_registration_covers_every_gauge() {
    let cache = RecordingCache::new();
    let batch = RecordingReleaseBatch::new();
    let (gauges, _script) = deferring_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let deferred = defer_slot_release(&gauges, &acquisition("slot-a", &["g1", "g2"])).await;
    assert!(deferred);
    let registrations = batch.registrations();
    let registration = registrations.first().expect("one registration");
    assert_eq!(registration.keys, key_list(&["g1", "g2"]));
    assert_eq!(
        registration.args,
        vec![
            RedisScalar::Text("slot-a".to_owned()),
            RedisScalar::Text("slot-a".to_owned()),
        ]
    );
}

#[tokio::test]
async fn no_batch_registers_nothing() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW)]));
    let (gauges, script) = deferring_gauges(Arc::clone(&cache), None);
    let deferred = defer_slot_release(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert!(!deferred);
    assert!(script.calls().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn no_release_script_registers_nothing() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW)]));
    let batch = RecordingReleaseBatch::new();
    let mut gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let shared: Arc<dyn PostCallScriptBatch> = Arc::<RecordingReleaseBatch>::clone(&batch);
    gauges.release_batch = Some(shared);
    let deferred = defer_slot_release(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert!(!deferred);
    assert!(batch.registrations().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn empty_slot_id_registers_nothing() {
    let cache = RecordingCache::new();
    let batch = RecordingReleaseBatch::new();
    let (gauges, _script) = deferring_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let deferred = defer_slot_release(&gauges, &acquisition("", &["g1"])).await;
    assert!(!deferred);
    assert!(batch.registrations().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn no_keys_register_nothing() {
    let cache = RecordingCache::new();
    let batch = RecordingReleaseBatch::new();
    let (gauges, _script) = deferring_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let deferred = defer_slot_release(&gauges, &acquisition("slot-a", &[])).await;
    assert!(!deferred);
    assert!(batch.registrations().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn batch_failure_logs_without_raising() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW)]));
    let batch = RecordingReleaseBatch::new();
    let (gauges, _script) = deferring_gauges(Arc::clone(&cache), Some(Arc::clone(&batch)));
    let deferred = defer_slot_release(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert!(deferred);
    let registrations = batch.registrations();
    let registration = registrations.first().expect("one registration");
    let capture = LogCapture::start();
    let failure: Settled<ScriptReply> = Err(Arc::new(batch_failure()));
    (registration.hook)(&failure);
    assert_eq!(
        capture
            .text()
            .matches("the slot stays released in memory only")
            .count(),
        1
    );
    let success: Settled<ScriptReply> = Ok(ScriptReply::Array(Vec::new()));
    (registration.hook)(&success);
    assert_eq!(
        capture
            .text()
            .matches("the slot stays released in memory only")
            .count(),
        1
    );
}
