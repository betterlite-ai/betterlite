#![cfg(test)]

use std::future::Future as _;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Waker};

use rstest::rstest;

use super::in_memory_cache_sliding_window;
use crate::limiter_records::CacheCounterValue;
use crate::rate_limit_window_store::RedisCacheKind;
use crate::rate_limit_window_store::WindowStoreError;
use crate::rate_limit_window_store::tests::support::{CacheWrite, FakeCache, key_list, store_with};

const WINDOW_TTL: i64 = 60;

fn pair() -> Vec<String> {
    key_list(&["{key:value}:window", "{key:value}:requests"])
}

fn window_key() -> String {
    "{key:value}:window".to_owned()
}

fn counter_key() -> String {
    "{key:value}:requests".to_owned()
}

#[tokio::test]
async fn a_new_window_starts_at_now_and_counts_one() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    let values = in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect("the sliding window pass");
    assert_eq!(
        values,
        vec![
            Some(CacheCounterValue::Text("100".to_owned())),
            Some(CacheCounterValue::Int(1)),
        ]
    );
}

#[tokio::test]
async fn an_expired_window_resets_the_counter_and_writes_the_window() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let store = store_with(Arc::clone(&cache));
    in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect("the sliding window pass");
    assert_eq!(
        cache.writes(),
        vec![
            CacheWrite {
                key: counter_key(),
                value: CacheCounterValue::Int(0),
                ttl: WINDOW_TTL,
                local_only: true,
            },
            CacheWrite {
                key: "{key:value}:tokens".to_owned(),
                value: CacheCounterValue::Int(0),
                ttl: WINDOW_TTL,
                local_only: true,
            },
            CacheWrite {
                key: window_key(),
                value: CacheCounterValue::Text("100".to_owned()),
                ttl: WINDOW_TTL,
                local_only: true,
            },
            CacheWrite {
                key: counter_key(),
                value: CacheCounterValue::Int(1),
                ttl: WINDOW_TTL,
                local_only: true,
            },
        ]
    );
}

#[tokio::test]
async fn a_live_window_increments_the_counter_only() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed(&window_key(), CacheCounterValue::Text("90".to_owned()));
    cache.seed(&counter_key(), CacheCounterValue::Int(2));
    let store = store_with(Arc::clone(&cache));
    let values = in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect("the sliding window pass");
    assert_eq!(
        values,
        vec![
            Some(CacheCounterValue::Text("90".to_owned())),
            Some(CacheCounterValue::Int(3)),
        ]
    );
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: counter_key(),
            value: CacheCounterValue::Int(3),
            ttl: WINDOW_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_live_window_without_a_counter_counts_the_first_request() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed(&window_key(), CacheCounterValue::Text("90".to_owned()));
    let store = store_with(Arc::clone(&cache));
    let values = in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect("the sliding window pass");
    assert_eq!(
        values,
        vec![
            Some(CacheCounterValue::Text("90".to_owned())),
            Some(CacheCounterValue::Int(1)),
        ]
    );
    assert_eq!(
        cache.writes(),
        vec![CacheWrite {
            key: counter_key(),
            value: CacheCounterValue::Int(1),
            ttl: WINDOW_TTL,
            local_only: true,
        }]
    );
}

#[tokio::test]
async fn a_pass_waits_for_the_check_and_increment_lock() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let store = store_with(Arc::clone(&cache));
    let guard = store.check_and_increment_lock.lock().await;
    let keys = pair();
    let pass = in_memory_cache_sliding_window(&store, &keys, 100, WINDOW_TTL);
    let mut pinned = pin!(pass);
    let mut context = Context::from_waker(Waker::noop());
    assert!(pinned.as_mut().poll(&mut context).is_pending());
    assert!(cache.gets().is_empty());
    drop(guard);
    let values = pinned.await.expect("the sliding window pass");
    assert_eq!(
        values,
        vec![
            Some(CacheCounterValue::Text("100".to_owned())),
            Some(CacheCounterValue::Int(1)),
        ]
    );
    assert_eq!(cache.gets().len(), 1);
}

#[rstest]
#[case::one_second_before_the_end(41, 2)]
#[case::exactly_at_the_window_size(40, 1)]
#[case::past_the_window_size(39, 1)]
#[tokio::test]
async fn a_window_expires_once_the_elapsed_time_reaches_its_size(
    #[case] window_start: i64,
    #[case] counter: i64,
) {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed(
        &window_key(),
        CacheCounterValue::Text(window_start.to_string()),
    );
    cache.seed(&counter_key(), CacheCounterValue::Int(1));
    let store = store_with(Arc::clone(&cache));
    let values = in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect("the sliding window pass");
    assert_eq!(values[1], Some(CacheCounterValue::Int(counter)));
}

#[tokio::test]
async fn every_window_read_and_write_stays_local() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed(&window_key(), CacheCounterValue::Text("90".to_owned()));
    cache.seed(&counter_key(), CacheCounterValue::Int(2));
    let store = store_with(Arc::clone(&cache));
    in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect("the sliding window pass");
    assert_eq!(
        cache.gets(),
        vec![(window_key(), true), (counter_key(), true)]
    );
}

#[tokio::test]
async fn a_key_list_that_is_not_paired_is_an_error() {
    let store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    let error = in_memory_cache_sliding_window(&store, &key_list(&["{key:value}:window"]), 100, 60)
        .await
        .expect_err("an unpaired key list");
    assert!(matches!(error, WindowStoreError::UnpairedKeys { count: 1 }));
}

#[tokio::test]
async fn a_window_start_that_is_not_a_whole_number_is_an_error() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed(&window_key(), CacheCounterValue::Text("soon".to_owned()));
    let store = store_with(Arc::clone(&cache));
    let error = in_memory_cache_sliding_window(&store, &pair(), 100, WINDOW_TTL)
        .await
        .expect_err("a window start that is not a number");
    assert!(matches!(error, WindowStoreError::WindowStart(_)));
}
