#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, LogCapture, PlannedScript, RecordingCache, ReplyPlan, acquisition,
    gauges_with, int_counter, int_values, key_list, registry,
};
use crate::concurrency_gauges::{ConcurrencyGauges, PARALLEL_SLOT_TTL};
use crate::limiter_records::CacheCounterValue;
use crate::rate_limit_window_store::CounterScript;

use super::release_request_slots;

const NOW: f64 = 1_700_000_000.0;

fn release_gauges(
    cache: Arc<RecordingCache>,
    plans: Vec<ReplyPlan>,
) -> (ConcurrencyGauges, Arc<PlannedScript>) {
    let mut gauges = gauges_with(cache, FixedClock::at(NOW));
    let handle = PlannedScript::new("release", plans);
    let script: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(&handle);
    gauges.release_script = Some(script);
    (gauges, handle)
}

#[tokio::test]
async fn success_mirrors_the_remaining_counts() {
    let cache = RecordingCache::new();
    let (gauges, script) = release_gauges(Arc::clone(&cache), vec![int_values(&[2])]);
    release_request_slots(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert_eq!(
        script.calls(),
        vec![(
            key_list(&["g1"]),
            vec![RedisScalar::Text("slot-a".to_owned())],
        )]
    );
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: int_counter(2),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn success_mirrors_every_gauge_count() {
    let cache = RecordingCache::new();
    let (gauges, _script) = release_gauges(Arc::clone(&cache), vec![int_values(&[1, 0])]);
    release_request_slots(&gauges, &acquisition("slot-a", &["g1", "g2"])).await;
    assert_eq!(
        cache.writes(),
        vec![
            CacheWrite {
                key: "g1".to_owned(),
                value: int_counter(1),
                ttl: PARALLEL_SLOT_TTL,
                local_only: true,
            },
            CacheWrite {
                key: "g2".to_owned(),
                value: int_counter(0),
                ttl: PARALLEL_SLOT_TTL,
                local_only: true,
            },
        ]
    );
}

#[tokio::test]
async fn fallback_to_in_memory_when_the_script_fails() {
    let capture = LogCapture::start();
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let (gauges, script) = release_gauges(Arc::clone(&cache), vec![ReplyPlan::Failure]);
    release_request_slots(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert_eq!(script.calls().len(), 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-b", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
    assert!(capture.text().contains("parallel_release_script failed"));
}

#[tokio::test]
async fn a_release_reply_that_is_not_an_array_falls_back_to_in_memory() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let (gauges, _script) = release_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Element(CacheCounterValue::Int(0))],
    );
    release_request_slots(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-b", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_release_count_that_is_not_whole_writes_nothing() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW)]));
    let (gauges, _script) = release_gauges(
        Arc::clone(&cache),
        vec![ReplyPlan::Values(vec![CacheCounterValue::Text(
            "soon".to_owned(),
        )])],
    );
    release_request_slots(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert!(cache.writes().is_empty());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn no_release_script_releases_in_memory() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_request_slots(&gauges, &acquisition("slot-a", &["g1"])).await;
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-b", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn empty_keys_release_nothing() {
    let cache = RecordingCache::new();
    let (gauges, script) = release_gauges(Arc::clone(&cache), vec![int_values(&[0])]);
    release_request_slots(&gauges, &acquisition("slot-a", &[])).await;
    assert!(script.calls().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn empty_slot_id_releases_nothing() {
    let cache = RecordingCache::new();
    let (gauges, script) = release_gauges(Arc::clone(&cache), vec![int_values(&[0])]);
    release_request_slots(&gauges, &acquisition("", &["g1"])).await;
    assert!(script.calls().is_empty());
    assert!(cache.writes().is_empty());
}
