#![cfg(test)]

use std::sync::Arc;

use crate::concurrency_gauges::PARALLEL_SLOT_TTL;
use crate::concurrency_gauges::tests::support::{
    CacheWrite, FixedClock, RecordingCache, gauges_with, int_counter, key_list,
};
use crate::limiter_records::CacheCounterValue;

use super::mirror_released_slots;

const NOW: f64 = 1_700_000_000.0;

#[tokio::test]
async fn stores_floored_counts_with_the_slot_ttl() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let remaining = [CacheCounterValue::Int(2), CacheCounterValue::Int(-1)];
    mirror_released_slots(&gauges, &key_list(&["g1", "g2"]), &remaining).await;
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
async fn skips_text_that_is_not_whole() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let remaining = [
        CacheCounterValue::Text("soon".to_owned()),
        CacheCounterValue::Int(3),
    ];
    mirror_released_slots(&gauges, &key_list(&["g1", "g2"]), &remaining).await;
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: "g2".to_owned(),
            value: int_counter(3),
            ttl: PARALLEL_SLOT_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn zips_keys_with_the_returned_counts() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let remaining = [CacheCounterValue::Int(1)];
    mirror_released_slots(&gauges, &key_list(&["g1", "g2"]), &remaining).await;
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
async fn a_float_count_is_cut_to_a_whole_number() {
    let cache = RecordingCache::new();
    let gauges = gauges_with(Arc::clone(&cache), FixedClock::at(NOW));
    let remaining = [CacheCounterValue::Float(2.9)];
    mirror_released_slots(&gauges, &key_list(&["g1"]), &remaining).await;
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
