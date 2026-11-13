#![cfg(test)]

use std::sync::Arc;

use super::batch_get_counter_values;
use crate::limiter_records::CacheCounterValue;
use crate::rate_limit_window_store::RedisCacheKind;
use crate::rate_limit_window_store::tests::support::{FakeCache, key_list, store_with};

#[tokio::test]
async fn the_batch_read_returns_one_value_per_key() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed("alpha", CacheCounterValue::Int(2));
    let store = store_with(Arc::clone(&cache));
    let keys = key_list(&["alpha", "beta"]);
    assert_eq!(
        batch_get_counter_values(&store, &keys, false).await,
        Some(vec![Some(CacheCounterValue::Int(2)), None])
    );
}

#[tokio::test]
async fn the_local_only_flag_reaches_the_cache() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let store = store_with(Arc::clone(&cache));
    let keys = key_list(&["alpha"]);
    assert_eq!(
        batch_get_counter_values(&store, &keys, true).await,
        Some(vec![None])
    );
    assert_eq!(cache.batch_reads(), vec![(keys, true)]);
}

#[tokio::test]
async fn a_failed_batch_read_is_none() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.fail_batch_reads();
    let store = store_with(Arc::clone(&cache));
    assert_eq!(
        batch_get_counter_values(&store, &key_list(&["alpha"]), true).await,
        None
    );
}

#[tokio::test]
async fn a_batch_read_with_a_value_for_every_other_key_is_none() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.set_batch_reply(vec![Some(CacheCounterValue::Int(1))]);
    let store = store_with(Arc::clone(&cache));
    assert_eq!(
        batch_get_counter_values(&store, &key_list(&["alpha", "beta"]), true).await,
        None
    );
}
