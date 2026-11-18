#![cfg(test)]

use std::sync::Arc;

use crate::concurrency_gauges::cache::GaugeCacheValue;
use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, RecordingCache, gauge, gauges_with, int_counter, registry,
};
use crate::concurrency_gauges::{GaugeError, PARALLEL_SLOT_TTL};
use crate::limiter_records::CacheCounterValue;

use super::acquire_slots_in_memory;

const NOW: f64 = 1_700_000_000.0;

#[tokio::test]
async fn dict_registry_adds_the_slot_and_counts_fresh_entries() {
    let cache = RecordingCache::new();
    cache.seed(
        "g1",
        registry(&[
            ("slot-old", 1_699_999_000.0),
            ("slot-stale", 1_699_000_000.0),
        ]),
    );
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 3)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    assert_eq!(response.overall_code, "OK");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-a", NOW), ("slot-old", 1_699_999_000.0)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn dict_registry_at_the_limit_is_a_rejection_without_a_write() {
    let cache = RecordingCache::new();
    cache.seed(
        "g1",
        registry(&[("slot-a", NOW), ("slot-b", NOW), ("slot-c", NOW)]),
    );
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 3)], "slot-d")
        .await
        .expect("an over-limit acquisition");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one rejecting status");
    assert_eq!(status.current_limit, 3);
    assert_eq!(status.limit_remaining, 0);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn integer_mirror_increments_by_one() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(4));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 6)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: int_counter(5),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn integer_mirror_below_zero_reads_as_empty() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(-2));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 5)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 4);
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
async fn all_or_nothing_writes_every_gauge_after_the_checks() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(1));
    cache.seed("g2", registry(&[]));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 5), gauge("g2", 5)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    let remaining: Vec<i64> = response
        .statuses
        .iter()
        .map(|status| status.limit_remaining)
        .collect();
    assert_eq!(remaining, vec![3, 4]);
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
                value: registry(&[("slot-a", NOW)]),
                ttl: PARALLEL_SLOT_TTL,
                local_only: true,
            },
        ]
    );
}

#[tokio::test]
async fn all_or_nothing_leaves_earlier_gauges_untouched_when_a_later_one_rejects() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(0));
    cache.seed("g2", int_counter(5));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 5), gauge("g2", 5)], "slot-a")
        .await
        .expect("an over-limit acquisition");
    assert_eq!(response.overall_code, "OVER_LIMIT");
    let status = response.statuses.first().expect("one rejecting status");
    assert_eq!(status.descriptor_key, "api-key");
    assert_eq!(status.limit_remaining, 0);
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn a_missing_cache_value_starts_a_new_registry() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[gauge("g1", 2)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    let status = response.statuses.first().expect("one gauge status");
    assert_eq!(status.limit_remaining, 1);
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g1".to_owned(),
            value: registry(&[("slot-a", NOW)]),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_mirror_that_is_not_a_whole_number_is_an_error() {
    let cache = RecordingCache::new();
    cache.seed(
        "g1",
        GaugeCacheValue::Counter(CacheCounterValue::Text("soon".to_owned())),
    );
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let error = acquire_slots_in_memory(&gauges, &[gauge("g1", 5)], "slot-a")
        .await
        .expect_err("a mirror that is not a number");
    assert!(matches!(error, GaugeError::NotWholeNumber { .. }));
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn every_gauge_is_read_once_through_the_local_tier() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    acquire_slots_in_memory(&gauges, &[gauge("g1", 5), gauge("g2", 5)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    assert_eq!(
        cache.reads(),
        vec![("g1".to_owned(), true), ("g2".to_owned(), true)]
    );
}

#[tokio::test]
async fn no_gauge_reports_ok_without_reading_the_cache() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let response = acquire_slots_in_memory(&gauges, &[], "slot-a")
        .await
        .expect("no gauges to acquire");
    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn keys_that_are_not_requested_are_left_alone() {
    let cache = RecordingCache::new();
    cache.seed("g2", int_counter(2));
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    acquire_slots_in_memory(&gauges, &[gauge("g1", 5)], "slot-a")
        .await
        .expect("an in-memory acquisition");
    assert_eq!(cache.reads(), vec![("g1".to_owned(), true)]);
    assert_eq!(cache.writes().len(), 1);
}
