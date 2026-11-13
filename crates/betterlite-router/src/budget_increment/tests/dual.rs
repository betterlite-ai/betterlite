use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use rust_decimal::Decimal;

use super::support::{Fixture, queued};
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::window::WindowCache as _;

fn thirty_seconds() -> Duration {
    Duration::from_secs(30)
}

#[tokio::test]
async fn a_memory_hit_skips_redis() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .cache
        .write_start_time("start:a", 42.0_f64, thirty_seconds())
        .await
        .unwrap();
    assert_eq!(
        fixture.cache.read_start_time("start:a").await.unwrap(),
        Some(42.0_f64)
    );
    assert!(fixture.redis().read_keys().is_empty());
}

#[tokio::test]
async fn a_redis_hit_backfills_memory() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture.redis().seed_start("start:a", 42.0_f64);
    assert_eq!(
        fixture.cache.read_start_time("start:a").await.unwrap(),
        Some(42.0_f64)
    );
    assert_eq!(
        fixture.memory_value("start:a"),
        Some(CacheValue::Float(42.0_f64))
    );
    assert_eq!(fixture.memory_expiry("start:a"), Some(1_600.0_f64));
}

#[tokio::test]
async fn a_redis_miss_leaves_memory_empty() {
    let fixture = Fixture::new(1_000.0_f64);
    assert_eq!(
        fixture.cache.read_start_time("start:a").await.unwrap(),
        None
    );
    assert!(fixture.memory_value("start:a").is_none());
}

#[tokio::test]
async fn a_redis_read_error_is_propagated() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture.redis().read_error(true);
    let error = fixture.cache.read_start_time("start:a").await.unwrap_err();
    assert!(matches!(error, IncrementError::InvalidCacheValue { key } if key == "start:a"));
    assert!(fixture.memory_value("start:a").is_none());
}

#[tokio::test]
async fn writes_reach_memory_and_redis() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .cache
        .write_start_time("start:a", 42.0_f64, thirty_seconds())
        .await
        .unwrap();
    assert_eq!(
        fixture.redis().start_writes(),
        vec![("start:a".to_owned(), 42.0_f64, thirty_seconds())]
    );
    assert_eq!(
        fixture.memory_value("start:a"),
        Some(CacheValue::Float(42.0_f64))
    );
    assert_eq!(fixture.memory_expiry("start:a"), Some(1_030.0_f64));
}

#[tokio::test]
async fn reset_spend_writes_memory_and_redis() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .cache
        .reset_spend("spend:a", Decimal::new(15, 1), thirty_seconds())
        .await
        .unwrap();
    assert_eq!(
        fixture.redis().spend_writes(),
        vec![("spend:a".to_owned(), Decimal::new(15, 1), thirty_seconds())]
    );
    assert!(fixture.memory_value("spend:a").is_some());
    assert_eq!(fixture.memory_expiry("spend:a"), Some(1_030.0_f64));
    let total = fixture
        .cache
        .increment_spend("spend:a", Decimal::new(5, 1), thirty_seconds())
        .await
        .unwrap();
    assert_eq!(total, Decimal::new(20, 1));
}

#[tokio::test]
async fn increment_spend_sums_exactly_in_memory() {
    let fixture = Fixture::new(1_000.0_f64);
    let first = fixture
        .cache
        .increment_spend("spend:a", Decimal::new(1, 1), thirty_seconds())
        .await
        .unwrap();
    assert_eq!(first, Decimal::new(1, 1));
    let second = fixture
        .cache
        .increment_spend("spend:a", Decimal::new(2, 1), thirty_seconds())
        .await
        .unwrap();
    assert_eq!(second, Decimal::new(3, 1));
    assert!(fixture.redis().spend_writes().is_empty());
    assert_eq!(fixture.memory_expiry("spend:a"), Some(1_030.0_f64));
}

#[tokio::test]
async fn increment_spend_starts_from_zero() {
    let fixture = Fixture::new(1_000.0_f64);
    let total = fixture
        .cache
        .increment_spend("spend:a", Decimal::new(25, 2), thirty_seconds())
        .await
        .unwrap();
    assert_eq!(total, Decimal::new(25, 2));
}

#[tokio::test]
async fn a_wrong_typed_memory_start_is_an_error() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .memory
        .lock()
        .unwrap()
        .set("start:a", CacheValue::Text("nope".to_owned()), None)
        .unwrap();
    let error = fixture.cache.read_start_time("start:a").await.unwrap_err();
    assert!(matches!(error, IncrementError::InvalidCacheValue { key } if key == "start:a"));
}

#[tokio::test]
async fn a_wrong_typed_memory_spend_is_an_error() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture
        .memory
        .lock()
        .unwrap()
        .set("spend:a", CacheValue::Text("nope".to_owned()), None)
        .unwrap();
    let error = fixture
        .cache
        .increment_spend("spend:a", Decimal::new(25, 2), thirty_seconds())
        .await
        .unwrap_err();
    assert!(matches!(error, IncrementError::InvalidCacheValue { key } if key == "spend:a"));
}

#[tokio::test]
async fn write_increments_without_redis_is_true() {
    let fixture = Fixture::memory_only(1_000.0_f64);
    assert!(
        fixture
            .cache
            .write_increments(&[queued("spend:a", Decimal::new(25, 2), 30)])
            .await
    );
}

#[tokio::test]
async fn write_increments_delegates_to_redis() {
    let fixture = Fixture::new(1_000.0_f64);
    fixture.redis().fail(true);
    assert!(
        !fixture
            .cache
            .write_increments(&[queued("spend:a", Decimal::new(25, 2), 30)])
            .await
    );
    fixture.redis().fail(false);
    assert!(
        fixture
            .cache
            .write_increments(&[queued("spend:a", Decimal::new(25, 2), 30)])
            .await
    );
    assert_eq!(
        fixture.redis().batches(),
        vec![vec![queued("spend:a", Decimal::new(25, 2), 30)]]
    );
}
