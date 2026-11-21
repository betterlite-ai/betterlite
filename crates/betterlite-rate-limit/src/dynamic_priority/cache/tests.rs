#![cfg(test)]

use super::saturation_value_from_cache;
use crate::dynamic_priority::tests::support::{CacheRead, RecordingCache};
use crate::limiter_records::CacheCounterValue;

const COUNTER_KEY: &str = "{model_saturation_check:gpt-4}:requests";

#[tokio::test]
async fn a_seeded_counter_comes_back_with_the_read_flags() {
    let cache = RecordingCache::seeded(&[(COUNTER_KEY, CacheCounterValue::Text("50".to_owned()))]);
    let value = saturation_value_from_cache(&cache, COUNTER_KEY, 60).await;
    assert_eq!(value, Some(CacheCounterValue::Text("50".to_owned())));
    assert_eq!(
        cache.reads(),
        vec![CacheRead {
            key: COUNTER_KEY.to_owned(),
            local_only: false,
            local_ttl: 60,
        }]
    );
}

#[tokio::test]
async fn a_missing_counter_reads_as_none() {
    let cache = RecordingCache::default();
    assert_eq!(
        saturation_value_from_cache(&cache, "absent", 60).await,
        None
    );
    assert_eq!(cache.reads().len(), 1);
}
