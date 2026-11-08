#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;

use super::memory::{DEFAULT_MAX_SIZE_PER_ITEM_KIB, MemoryCache};
use super::{
    BASE_DEFAULT_TTL_SECONDS, BackendError, CacheBackend, ConnectionStatus, ConnectionTest,
};
use crate::types::pipeline::RedisPipelineIncrement;

struct BaseContract;

impl CacheBackend for BaseContract {
    fn default_ttl(&self) -> i64 {
        BASE_DEFAULT_TTL_SECONDS
    }
}

fn clock_at(seconds: f64) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(move || seconds)
}

fn cache_at(seconds: f64) -> MemoryCache {
    MemoryCache::new(
        None,
        None,
        None,
        DEFAULT_MAX_SIZE_PER_ITEM_KIB,
        clock_at(seconds),
    )
    .expect("the default memory cache")
}

fn mutable_clock() -> (Arc<AtomicU64>, Arc<dyn Fn() -> f64 + Send + Sync>) {
    let ticks = Arc::new(AtomicU64::new(0.0_f64.to_bits()));
    let source = Arc::clone(&ticks);
    let clock = Arc::new(move || f64::from_bits(source.load(Ordering::Relaxed)));
    (ticks, clock)
}

fn fill(cache: &MemoryCache, count: usize) {
    for index in 0..count {
        let step = u32::try_from(index)
            .expect("small index")
            .checked_add(1)
            .expect("small index");
        let ttl = f64::from(step);
        cache
            .set_cache(&format!("key-{index}"), CacheValue::Integer(1), Some(ttl))
            .expect("a stored key");
    }
}

#[rstest]
#[case::absent(None, 60)]
#[case::explicit(Some(15), 15)]
#[case::zero(Some(0), 0)]
fn base_get_ttl_precedes_the_default(#[case] ttl: Option<i64>, #[case] expected: i64) {
    assert_eq!(BaseContract.get_ttl(ttl), expected);
}

#[test]
fn base_get_cache_is_unsupported() {
    let error = BaseContract.get_cache("key").expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported {
            operation: "get_cache"
        }
    ));
}

#[test]
fn base_set_cache_is_unsupported() {
    let error = BaseContract
        .set_cache("key", CacheValue::Integer(1), None)
        .expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported {
            operation: "set_cache"
        }
    ));
}

#[tokio::test]
async fn base_test_connection_is_unsupported() {
    let error = BaseContract
        .test_connection()
        .await
        .expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported {
            operation: "test_connection"
        }
    ));
}

#[tokio::test]
async fn base_disconnect_is_unsupported() {
    let error = BaseContract.disconnect().await.expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported {
            operation: "disconnect"
        }
    ));
}

#[tokio::test]
async fn base_pipeline_reads_and_writes_are_unsupported() {
    let error = BaseContract
        .async_set_cache_pipeline(Vec::new(), None)
        .await
        .expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported {
            operation: "async_set_cache_pipeline"
        }
    ));
    let batch_error = BaseContract
        .async_batch_get_cache(&[])
        .await
        .expect_err("unsupported");
    assert!(matches!(
        batch_error,
        BackendError::Unsupported {
            operation: "async_batch_get_cache"
        }
    ));
}

#[test]
fn memory_default_ttl_is_six_hundred() {
    assert_eq!(cache_at(0.0_f64).default_ttl(), 600);
}

#[tokio::test]
async fn memory_reads_what_the_sync_cache_stored() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Text("value".to_owned()), None)
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_cache("key").await.expect("a read"),
        Some(CacheValue::Text("value".to_owned()))
    );
    assert_eq!(
        cache.async_get_cache("missing").await.expect("a read"),
        None
    );
}

#[tokio::test]
async fn memory_get_ttl_returns_the_absolute_expiry() {
    let cache = cache_at(1000.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(1030.0_f64)
    );
    assert_eq!(cache.async_get_ttl("missing").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_get_ttl_uses_the_default_without_an_explicit_ttl() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), None)
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(600.0_f64)
    );
}

#[tokio::test]
async fn memory_read_allows_a_key_at_its_expiry() {
    let (ticks, clock) = mutable_clock();
    let cache = MemoryCache::new(None, None, None, DEFAULT_MAX_SIZE_PER_ITEM_KIB, clock)
        .expect("the default memory cache");
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(10.0_f64))
        .await
        .expect("a stored value");
    ticks.store(10.0_f64.to_bits(), Ordering::Relaxed);
    assert_eq!(
        cache.async_get_cache("key").await.expect("a read"),
        Some(CacheValue::Integer(1))
    );
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(10.0_f64)
    );
    ticks.store(10.5_f64.to_bits(), Ordering::Relaxed);
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_batch_get_preserves_order_and_misses() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("a", CacheValue::Integer(1), None)
        .await
        .expect("a stored value");
    cache
        .async_set_cache("c", CacheValue::Integer(3), None)
        .await
        .expect("a stored value");
    let keys = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
    assert_eq!(
        cache.async_batch_get_cache(&keys).await.expect("a batch"),
        vec![
            Some(CacheValue::Integer(1)),
            None,
            Some(CacheValue::Integer(3))
        ]
    );
}

#[tokio::test]
async fn memory_set_pipeline_applies_the_ttl() {
    let cache = cache_at(0.0_f64);
    let entries = vec![
        ("a".to_owned(), CacheValue::Integer(1)),
        ("b".to_owned(), CacheValue::Integer(2)),
    ];
    cache
        .async_set_cache_pipeline(entries, Some(30.0_f64))
        .await
        .expect("a pipeline write");
    assert_eq!(
        cache.async_get_ttl("a").await.expect("a ttl"),
        Some(30.0_f64)
    );
    assert_eq!(
        cache.async_get_ttl("b").await.expect("a ttl"),
        Some(30.0_f64)
    );
}

#[tokio::test]
async fn memory_increment_starts_from_zero() {
    let cache = cache_at(0.0_f64);
    let incremented = cache
        .async_increment("key", 1.5_f64, None)
        .await
        .expect("an increment");
    assert_eq!(incremented.to_bits(), 1.5_f64.to_bits());
    assert_eq!(
        cache.async_get_cache("key").await.expect("a read"),
        Some(CacheValue::Float(1.5_f64))
    );
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(600.0_f64)
    );
}

#[tokio::test]
async fn memory_increment_adds_to_the_stored_value() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("float", CacheValue::Float(2.0_f64), None)
        .await
        .expect("a stored value");
    cache
        .async_set_cache("int", CacheValue::Integer(2), None)
        .await
        .expect("a stored value");
    let from_float = cache
        .async_increment("float", 1.5_f64, None)
        .await
        .expect("an increment");
    let from_integer = cache
        .async_increment("int", 1.5_f64, None)
        .await
        .expect("an increment");
    assert_eq!(from_float.to_bits(), 3.5_f64.to_bits());
    assert_eq!(from_integer.to_bits(), 3.5_f64.to_bits());
}

#[tokio::test]
async fn memory_increment_rejects_a_non_number() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Text("text".to_owned()), None)
        .await
        .expect("a stored value");
    let error = cache
        .async_increment("key", 1.0_f64, None)
        .await
        .expect_err("a non-number");
    assert!(matches!(error, BackendError::NotNumeric { key } if key == "key"));
}

#[tokio::test]
async fn memory_increment_rejects_a_non_finite_result() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Float(f64::MAX), None)
        .await
        .expect("a stored value");
    let error = cache
        .async_increment("key", f64::MAX, None)
        .await
        .expect_err("an overflow");
    assert!(matches!(
        error,
        BackendError::IncrementOutOfRange { key } if key == "key"
    ));
}

#[tokio::test]
async fn memory_increment_pipeline_returns_each_result() {
    let cache = cache_at(0.0_f64);
    let increments = vec![
        RedisPipelineIncrement {
            key: "a".to_owned(),
            increment_value: 1.5_f64,
            ttl: Some(5),
        },
        RedisPipelineIncrement {
            key: "a".to_owned(),
            increment_value: 2.0_f64,
            ttl: None,
        },
        RedisPipelineIncrement {
            key: "b".to_owned(),
            increment_value: 1.0_f64,
            ttl: None,
        },
    ];
    assert_eq!(
        cache
            .async_increment_pipeline(increments)
            .await
            .expect("a pipeline"),
        vec![1.5_f64, 3.5_f64, 1.0_f64]
    );
    assert_eq!(
        cache.async_get_cache("a").await.expect("a read"),
        Some(CacheValue::Float(3.5_f64))
    );
    // LiteLLM's pipeline hands each record to async_increment without its ttl, so the memory default applies
    assert_eq!(
        cache.async_get_ttl("a").await.expect("a ttl"),
        Some(600.0_f64)
    );
}

#[tokio::test]
async fn memory_oldest_keys_orders_by_expiry() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("b", CacheValue::Integer(1), Some(20.0_f64))
        .await
        .expect("a stored value");
    cache
        .async_set_cache("a", CacheValue::Integer(1), Some(10.0_f64))
        .await
        .expect("a stored value");
    cache
        .async_set_cache("c", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache
            .async_get_oldest_n_keys(2)
            .await
            .expect("the oldest keys"),
        vec!["a".to_owned(), "b".to_owned()]
    );
    assert_eq!(
        cache
            .async_get_oldest_n_keys(10)
            .await
            .expect("the oldest keys"),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
    assert_eq!(
        cache
            .async_get_oldest_n_keys(0)
            .await
            .expect("the oldest keys"),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn memory_delete_removes_the_value_and_the_ttl() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    cache.delete_cache("key").expect("a delete");
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_flush_clears_the_values_and_the_ttls() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    cache.flush_cache().expect("a flush");
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key").await.expect("a ttl"), None);
    cache
        .async_set_cache("key", CacheValue::Integer(2), Some(10.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(10.0_f64)
    );
}

#[tokio::test]
async fn memory_disconnect_succeeds() {
    cache_at(0.0_f64).disconnect().await.expect("a disconnect");
}

#[tokio::test]
async fn memory_connection_test_is_unsupported() {
    let error = cache_at(0.0_f64)
        .test_connection()
        .await
        .expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported {
            operation: "test_connection"
        }
    ));
}

#[tokio::test]
async fn memory_capacity_evicts_the_earliest_expiry() {
    let cache = MemoryCache::new(
        Some(2),
        None,
        None,
        DEFAULT_MAX_SIZE_PER_ITEM_KIB,
        clock_at(0.0_f64),
    )
    .expect("a two-entry cache");
    cache
        .async_set_cache("a", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    cache
        .async_set_cache("b", CacheValue::Integer(1), Some(10.0_f64))
        .await
        .expect("a stored value");
    cache
        .async_set_cache("c", CacheValue::Integer(1), Some(20.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(cache.async_get_cache("b").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("b").await.expect("a ttl"), None);
    assert_eq!(
        cache.async_get_cache("a").await.expect("a read"),
        Some(CacheValue::Integer(1))
    );
    assert_eq!(
        cache.async_get_cache("c").await.expect("a read"),
        Some(CacheValue::Integer(1))
    );
}

#[tokio::test]
async fn memory_default_capacity_is_two_hundred() {
    let cache = cache_at(0.0_f64);
    fill(&cache, 201);
    assert_eq!(cache.async_get_cache("key-0").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key-0").await.expect("a ttl"), None);
    assert_eq!(
        cache.async_get_ttl("key-200").await.expect("a ttl"),
        Some(201.0_f64)
    );
}

#[tokio::test]
async fn memory_rejection_after_eviction_keeps_nothing() {
    let cache = MemoryCache::new(
        Some(1),
        None,
        Some(1),
        DEFAULT_MAX_SIZE_PER_ITEM_KIB,
        clock_at(0.0_f64),
    )
    .expect("a one-entry cache");
    cache
        .async_set_cache("a", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    cache
        .async_set_cache("b", CacheValue::Text("x".repeat(2000)), Some(30.0_f64))
        .await
        .expect("a rejected value");
    assert_eq!(cache.async_get_cache("a").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("a").await.expect("a ttl"), None);
    assert_eq!(cache.async_get_cache("b").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("b").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_zero_capacity_stores_nothing() {
    let cache = MemoryCache::new(
        Some(0),
        None,
        None,
        DEFAULT_MAX_SIZE_PER_ITEM_KIB,
        clock_at(0.0_f64),
    )
    .expect("a disabled cache");
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a refused value");
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_stored_value_decodes_like_the_shared_cache() {
    let cache = cache_at(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Text("{\"a\":1}".to_owned()), None)
        .await
        .expect("a stored value");
    let mut decoded = BTreeMap::new();
    decoded.insert("a".to_owned(), CacheValue::Integer(1));
    assert_eq!(
        cache.async_get_cache("key").await.expect("a read"),
        Some(CacheValue::Object(decoded))
    );
}

#[tokio::test]
async fn memory_connection_status_serializes_only_the_failure_error() {
    let success = ConnectionTest {
        status: ConnectionStatus::Success,
        message: "ok".to_owned(),
        error: None,
    };
    assert_eq!(
        serde_json::to_string(&success).expect("a wire form"),
        "{\"status\":\"success\",\"message\":\"ok\"}"
    );
    let failure = ConnectionTest {
        status: ConnectionStatus::Failed,
        message: "failed".to_owned(),
        error: Some("denied".to_owned()),
    };
    assert_eq!(
        serde_json::to_string(&failure).expect("a wire form"),
        "{\"status\":\"failed\",\"message\":\"failed\",\"error\":\"denied\"}"
    );
}
