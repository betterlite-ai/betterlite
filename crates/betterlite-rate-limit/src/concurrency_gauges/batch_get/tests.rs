#![cfg(test)]

use crate::concurrency_gauges::tests::support::{RecordingCache, int_counter, key_list, registry};

use super::batch_gauge_values;

#[tokio::test]
async fn cache_values_come_back_for_every_requested_key() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    cache.seed("g3", registry(&[("slot-a", 1.0)]));
    let values = batch_gauge_values(cache.as_ref(), &key_list(&["g1", "g2", "g3"])).await;
    assert_eq!(
        values,
        vec![
            Some(int_counter(2)),
            None,
            Some(registry(&[("slot-a", 1.0)])),
        ]
    );
}

#[tokio::test]
async fn a_failed_batch_read_gives_no_value_per_key() {
    let cache = RecordingCache::new();
    cache.seed("g1", int_counter(2));
    cache.fail_batch_reads();
    let values = batch_gauge_values(cache.as_ref(), &key_list(&["g1", "g2"])).await;
    assert_eq!(values, vec![None, None]);
}

#[tokio::test]
async fn the_batch_read_asks_for_the_local_tier_only() {
    let cache = RecordingCache::new();
    let keys = key_list(&["g1"]);
    let values = batch_gauge_values(cache.as_ref(), &keys).await;
    assert_eq!(values.len(), 1);
    assert_eq!(cache.batch_reads(), vec![(keys, true)]);
}
