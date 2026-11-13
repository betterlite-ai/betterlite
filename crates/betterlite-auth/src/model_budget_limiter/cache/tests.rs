#![cfg(test)]

use rust_decimal::Decimal;

use super::cached_spend;
use crate::model_budget_limiter::tests::support::{FakeCache, amount, limiter};
use crate::model_budget_values::spend_reads::CachedSpend;

#[tokio::test]
async fn redis_is_read_when_it_is_configured_and_memory_is_ignored() {
    let cache = FakeCache::with_redis();
    cache.preload("counter", amount("100"));
    cache.preload_redis("counter", amount("5"));
    let subject = limiter(cache);
    assert_eq!(cached_spend(&subject, "counter").await, amount("5"));
}

#[tokio::test]
async fn memory_is_read_when_no_redis_is_configured() {
    let cache = FakeCache::new();
    cache.preload("counter", amount("100"));
    let subject = limiter(cache);
    assert_eq!(cached_spend(&subject, "counter").await, amount("100"));
}

#[tokio::test]
async fn a_redis_miss_reads_as_zero_even_when_memory_holds_a_value() {
    let cache = FakeCache::with_redis();
    cache.preload("counter", amount("100"));
    let subject = limiter(cache);
    assert_eq!(cached_spend(&subject, "counter").await, Decimal::ZERO);
}

#[tokio::test]
async fn a_counter_that_was_never_written_reads_as_zero() {
    let subject = limiter(FakeCache::new());
    assert_eq!(cached_spend(&subject, "counter").await, Decimal::ZERO);
}

#[tokio::test]
async fn a_numeric_text_counter_reads_as_its_number() {
    let cache = FakeCache::new();
    cache.memory_values.lock().unwrap().insert(
        "counter".to_owned(),
        CachedSpend::Text("  3.5  ".to_owned()),
    );
    let subject = limiter(cache);
    assert_eq!(cached_spend(&subject, "counter").await, amount("3.5"));
}

#[tokio::test]
async fn a_non_finite_text_counter_reads_as_zero() {
    let cache = FakeCache::new();
    cache
        .memory_values
        .lock()
        .unwrap()
        .insert("counter".to_owned(), CachedSpend::Text("-inf".to_owned()));
    let subject = limiter(cache);
    assert_eq!(cached_spend(&subject, "counter").await, Decimal::ZERO);
}

#[tokio::test]
async fn an_unreadable_counter_reads_as_zero() {
    let cache = FakeCache::new();
    cache
        .memory_values
        .lock()
        .unwrap()
        .insert("counter".to_owned(), CachedSpend::Unreadable);
    let subject = limiter(cache);
    assert_eq!(cached_spend(&subject, "counter").await, Decimal::ZERO);
}
