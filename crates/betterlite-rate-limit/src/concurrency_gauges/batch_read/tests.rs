#![cfg(test)]

use std::sync::Arc;

use crate::concurrency_gauges::GaugeError;
use crate::concurrency_gauges::cache::GaugeCacheValue;
use crate::concurrency_gauges::tests::support::{
    FixedClock, RecordingCache, gauges_with, int_counter, key_list, registry,
};
use crate::limiter_records::CacheCounterValue;

use super::read_local_counts;

const NOW: f64 = 1_700_000_000.0;

#[tokio::test]
async fn local_counts_come_from_the_mirrors_and_registries() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    cache.seed(
        "g2",
        registry(&[("slot-a", NOW), ("slot-stale", 1_699_000_000.0_f64)]),
    );
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let counts = read_local_counts(&gauges, &key_list(&["g1", "g2"]))
        .await
        .expect("local gauge counts");
    assert_eq!(counts, vec![2, 1]);
    assert_eq!(cache.batch_reads().len(), 1);
}

#[tokio::test]
async fn nil_batch_read_gives_zero_per_gauge() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    cache.fail_batch_reads();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let counts = read_local_counts(&gauges, &key_list(&["g1", "g2"]))
        .await
        .expect("local gauge counts");
    assert_eq!(counts, vec![0, 0]);
}

#[tokio::test]
async fn an_empty_gauge_list_needs_no_counts() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let counts = read_local_counts(&gauges, &[])
        .await
        .expect("local gauge counts");
    assert!(counts.is_empty());
    assert_eq!(cache.batch_reads(), vec![(Vec::new(), true)]);
}

#[tokio::test]
async fn an_unreadable_mirror_is_an_error() {
    let cache = RecordingCache::new();
    cache.seed(
        "g1",
        GaugeCacheValue::Counter(CacheCounterValue::Text("soon".to_owned())),
    );
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let error = read_local_counts(&gauges, &key_list(&["g1"]))
        .await
        .expect_err("a mirror that is not a number");
    assert!(matches!(error, GaugeError::NotWholeNumber { .. }));
}
