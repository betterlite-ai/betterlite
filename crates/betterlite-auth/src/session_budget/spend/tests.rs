#![cfg(test)]

use std::sync::atomic::Ordering;
use std::time::Duration;

use rust_decimal::Decimal;

use super::add_spend;
use super::current_spend;
use crate::session_budget::tests::support::FakeCache;
use crate::session_budget::tests::support::amount;
use crate::session_budget::tests::support::handler;
use crate::session_budget::tests::support::handler_at;

const KEY: &str = "{session_budget:s1}:spend";

#[tokio::test]
async fn in_memory_increment_adds_to_a_missing_key() {
    let handler = handler(FakeCache::new());
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
    assert_eq!(
        handler.cache.memory_ttl(KEY),
        Some(Duration::from_secs(3600))
    );
}

#[tokio::test]
async fn in_memory_increment_adds_to_the_existing_spend() {
    let cache = FakeCache::new();
    cache.preload_memory(KEY, amount("0.5"));
    let handler = handler(cache);
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.75")));
}

#[tokio::test]
async fn in_memory_increment_reads_a_text_spend() {
    let cache = FakeCache::new();
    cache.preload_memory_text(KEY, "0.5");
    let handler = handler(cache);
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.75")));
}

#[tokio::test]
async fn in_memory_increment_treats_an_unreadable_spend_as_zero() {
    let cache = FakeCache::new();
    cache.preload_memory_unreadable(KEY);
    let handler = handler(cache);
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
}

#[tokio::test]
async fn in_memory_increment_saturates_at_the_largest_decimal() {
    let cache = FakeCache::new();
    cache.preload_memory(KEY, Decimal::MAX);
    let handler = handler(cache);
    add_spend(&handler, KEY, amount("1")).await;
    assert_eq!(handler.cache.memory(KEY), Some(Decimal::MAX));
}

#[tokio::test]
async fn in_memory_increment_records_the_configured_ttl() {
    let handler = handler_at(FakeCache::new(), Duration::from_secs(120));
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(
        handler.cache.memory_ttl(KEY),
        Some(Duration::from_secs(120))
    );
}

#[tokio::test]
async fn in_memory_increment_runs_when_the_redis_cache_has_no_script() {
    let cache = FakeCache::with_redis();
    cache.holds_script.store(false, Ordering::SeqCst);
    let handler = handler(cache);
    assert!(handler.script.is_none());
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
    assert_eq!(handler.cache.redis(KEY), None);
}

#[tokio::test]
async fn in_memory_read_returns_the_stored_spend() {
    let cache = FakeCache::new();
    cache.preload_memory(KEY, amount("0.75"));
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("0.75"));
}

#[tokio::test]
async fn in_memory_read_is_zero_without_a_stored_spend() {
    let handler = handler(FakeCache::new());
    assert_eq!(current_spend(&handler, KEY).await, amount("0"));
}

#[tokio::test]
async fn in_memory_unreadable_value_reads_as_zero() {
    let cache = FakeCache::new();
    cache.preload_memory_unreadable(KEY);
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("0"));
}

#[tokio::test]
async fn redis_spend_wins_over_the_local_cache() {
    let cache = FakeCache::with_redis();
    cache.preload_redis(KEY, amount("0.5"));
    cache.preload_memory(KEY, amount("2.0"));
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("0.5"));
}

#[tokio::test]
async fn redis_spend_parses_a_text_value() {
    let cache = FakeCache::with_redis();
    cache.preload_redis_text(KEY, "0.75");
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("0.75"));
}

#[tokio::test]
async fn redis_missing_key_reads_as_zero_without_the_local_cache() {
    let cache = FakeCache::with_redis();
    cache.preload_memory(KEY, amount("5.0"));
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("0"));
}

#[tokio::test]
async fn redis_read_failure_uses_the_in_memory_fallback() {
    let cache = FakeCache::with_redis();
    cache.fails_redis_read.store(true, Ordering::SeqCst);
    cache.preload_memory(KEY, amount("1.25"));
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("1.25"));
}

#[tokio::test]
async fn redis_unreadable_value_uses_the_in_memory_fallback() {
    let cache = FakeCache::with_redis();
    cache.preload_redis_unreadable(KEY);
    cache.preload_memory(KEY, amount("1.25"));
    let handler = handler(cache);
    assert_eq!(current_spend(&handler, KEY).await, amount("1.25"));
}

#[tokio::test]
async fn lua_script_increment_writes_through_to_redis() {
    let handler = handler(FakeCache::with_redis());
    add_spend(&handler, KEY, amount("0.25")).await;
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.redis(KEY), Some(amount("0.5")));
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn lua_script_increment_carries_the_key_amount_and_ttl() {
    let handler = handler_at(FakeCache::with_redis(), Duration::from_secs(120));
    add_spend(&handler, KEY, amount("0.25")).await;
    let runs = handler.cache.script_runs.lock().unwrap();
    let run = runs.first().expect("the script ran");
    assert_eq!(run.key, KEY);
    assert_eq!(run.amount, amount("0.25"));
    assert_eq!(run.ttl, Duration::from_secs(120));
}

#[tokio::test]
async fn redis_increment_failure_uses_the_in_memory_fallback() {
    let cache = FakeCache::with_redis();
    cache.fails_script.store(true, Ordering::SeqCst);
    let handler = handler(cache);
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
    assert_eq!(handler.cache.redis(KEY), None);
}

#[tokio::test]
async fn redis_increment_failure_adds_to_the_existing_local_spend() {
    let cache = FakeCache::with_redis();
    cache.fails_script.store(true, Ordering::SeqCst);
    cache.preload_memory(KEY, amount("0.5"));
    let handler = handler(cache);
    add_spend(&handler, KEY, amount("0.25")).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.75")));
}
