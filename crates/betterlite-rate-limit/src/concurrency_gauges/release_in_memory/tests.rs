#![cfg(test)]

use std::sync::Arc;

use crate::concurrency_gauges::PARALLEL_SLOT_TTL;
use crate::concurrency_gauges::cache::GaugeCacheValue;
use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, RecordingCache, gauges_with, int_counter, key_list, registry,
};
use crate::limiter_records::CacheCounterValue;

use super::release_slots_in_memory;

const NOW: f64 = 1_700_000_000.0;

#[tokio::test]
async fn dict_removal_drops_only_this_slot() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW), ("slot-b", NOW)]));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1"]), "slot-a").await;
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
async fn unmatched_slot_leaves_the_registry_alone() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-b", NOW)]));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1"]), "slot-a").await;
    assert_eq!(cache.reads(), vec![("g1".to_owned(), true)]);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn integer_decrement_floors_at_zero() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(3));
    cache.seed("g2", int_counter(0));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1", "g2"]), "slot-a").await;
    assert_eq!(
        cache.writes(),
        vec![
            CacheWrite {
                key: "g1".to_owned(),
                value: int_counter(2),
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
async fn double_fired_release_when_the_cache_is_empty() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1"]), "slot-a").await;
    assert_eq!(cache.reads(), vec![("g1".to_owned(), true)]);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn double_fired_release_of_a_removed_slot_writes_nothing_more() {
    let cache = RecordingCache::new();
    cache.seed("g1", registry(&[("slot-a", NOW)]));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1"]), "slot-a").await;
    release_slots_in_memory(&gauges, &key_list(&["g1"]), "slot-a").await;
    assert_eq!(cache.writes().len(), 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_mirror_that_is_not_a_whole_number_is_skipped() {
    let cache = RecordingCache::new();
    cache.seed(
        "g1",
        GaugeCacheValue::Counter(CacheCounterValue::Text("soon".to_owned())),
    );
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1"]), "slot-a").await;
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn every_key_is_released_in_one_pass() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(1));
    cache.seed("g2", int_counter(2));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &key_list(&["g1", "g2"]), "slot-a").await;
    assert_eq!(
        cache.writes(),
        vec![
            CacheWrite {
                key: "g1".to_owned(),
                value: int_counter(0),
                ttl: PARALLEL_SLOT_TTL,
                local_only: true,
            },
            CacheWrite {
                key: "g2".to_owned(),
                value: int_counter(1),
                ttl: PARALLEL_SLOT_TTL,
                local_only: true,
            },
        ]
    );
}

#[tokio::test]
async fn no_keys_release_nothing() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    release_slots_in_memory(&gauges, &[], "slot-a").await;
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}
