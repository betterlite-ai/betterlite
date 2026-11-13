#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;
use serde_json::Number;

use super::memory::local::LocalMemoryOps;
use super::memory::{MemoryCache, MemoryCacheOps};
use super::{
    BASE_DEFAULT_TTL_SECONDS, BackendError, CacheBackend, CacheResult, ConnectionStatus,
    ConnectionTest, Ttl,
};
use crate::types::pipeline::RedisPipelineIncrement;

struct BaseContract;

impl CacheBackend for BaseContract {
    fn default_ttl(&self) -> i64 {
        BASE_DEFAULT_TTL_SECONDS
    }
}

#[derive(Default)]
struct FakeState {
    values: BTreeMap<String, CacheValue>,
    entries: Vec<(String, f64)>,
    set_ttls: Vec<(String, Ttl)>,
    increments: Vec<(String, f64, Ttl)>,
    default_ttl: i64,
    default_expiry: f64,
}

struct FakeOps {
    state: Mutex<FakeState>,
}

impl FakeOps {
    fn with_default_ttl(default_ttl: i64) -> Self {
        Self {
            state: Mutex::new(FakeState {
                values: BTreeMap::new(),
                entries: Vec::new(),
                set_ttls: Vec::new(),
                increments: Vec::new(),
                default_ttl,
                default_expiry: Number::from(default_ttl)
                    .as_f64()
                    .expect("a finite default"),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record_entry(&self, key: &str, ttl: Ttl) {
        let mut state = self.state();
        let expiry = ttl.unwrap_or(state.default_expiry);
        match state.entries.iter_mut().find(|(name, _)| name == key) {
            Some(entry) => entry.1 = expiry,
            None => state.entries.push((key.to_owned(), expiry)),
        }
    }
}

impl MemoryCacheOps for FakeOps {
    fn default_ttl(&self) -> i64 {
        self.state().default_ttl
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        Ok(self.state().values.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.state().values.insert(key.to_owned(), value);
        self.state().set_ttls.push((key.to_owned(), ttl));
        self.record_entry(key, ttl);
        Ok(())
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        self.state().values.remove(key);
        self.state().entries.retain(|(name, _)| name != key);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        self.state().values.clear();
        self.state().entries.clear();
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        let state = self.state();
        Ok(keys
            .iter()
            .map(|key| state.values.get(key).cloned())
            .collect())
    }

    fn increment_cache(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        let base = match self.state().values.get(key) {
            None => 0.0_f64,
            Some(CacheValue::Integer(number)) => {
                Number::from(*number)
                    .as_f64()
                    .ok_or_else(|| BackendError::NotNumeric {
                        key: key.to_owned(),
                    })?
            }
            Some(CacheValue::Float(number)) => *number,
            Some(_) => {
                return Err(BackendError::NotNumeric {
                    key: key.to_owned(),
                });
            }
        };
        let next = base.mul_add(1.0_f64, value);
        self.state()
            .values
            .insert(key.to_owned(), CacheValue::Float(next));
        self.state().increments.push((key.to_owned(), value, ttl));
        self.record_entry(key, ttl);
        Ok(next)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        Ok(self.state().entries.clone())
    }
}

fn cache(default_ttl: i64) -> MemoryCache<FakeOps> {
    MemoryCache {
        operations: FakeOps::with_default_ttl(default_ttl),
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
    assert!(matches!(error, BackendError::Unsupported("get_cache")));
}

#[test]
fn base_set_cache_is_unsupported() {
    let error = BaseContract
        .set_cache("key", CacheValue::Integer(1), None)
        .expect_err("unsupported");
    assert!(matches!(error, BackendError::Unsupported("set_cache")));
}

#[tokio::test]
async fn base_test_connection_is_unsupported() {
    let error = BaseContract
        .test_connection()
        .await
        .expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported("test_connection")
    ));
}

#[tokio::test]
async fn base_disconnect_is_unsupported() {
    let error = BaseContract.disconnect().await.expect_err("unsupported");
    assert!(matches!(error, BackendError::Unsupported("disconnect")));
}

#[tokio::test]
async fn base_pipeline_reads_and_writes_are_unsupported() {
    let error = BaseContract
        .async_set_cache_pipeline(Vec::new(), None)
        .await
        .expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported("async_set_cache_pipeline")
    ));
    let batch_error = BaseContract
        .async_batch_get_cache(&[])
        .await
        .expect_err("unsupported");
    assert!(matches!(
        batch_error,
        BackendError::Unsupported("async_batch_get_cache")
    ));
}

#[tokio::test]
async fn base_async_reads_and_writes_are_unsupported() {
    let read_error = BaseContract
        .async_get_cache("key")
        .await
        .expect_err("unsupported");
    assert!(matches!(
        read_error,
        BackendError::Unsupported("async_get_cache")
    ));
    let write_error = BaseContract
        .async_set_cache("key", CacheValue::Integer(1), None)
        .await
        .expect_err("unsupported");
    assert!(matches!(
        write_error,
        BackendError::Unsupported("async_set_cache")
    ));
    let batch_error = BaseContract
        .batch_cache_write("key", CacheValue::Integer(1), None)
        .await
        .expect_err("unsupported");
    assert!(matches!(
        batch_error,
        BackendError::Unsupported("batch_cache_write")
    ));
}

#[test]
fn memory_default_ttl_comes_from_the_operations() {
    assert_eq!(cache(600).default_ttl(), 600);
}

#[tokio::test]
async fn memory_reads_what_the_operations_stored() {
    let cache = cache(600);
    cache
        .async_set_cache("key", CacheValue::Text("value".to_owned()), Some(30.0_f64))
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
async fn memory_batch_get_preserves_order_and_misses() {
    let cache = cache(600);
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
    let cache = cache(600);
    let entries = vec![
        ("a".to_owned(), CacheValue::Integer(1)),
        ("b".to_owned(), CacheValue::Integer(2)),
    ];
    cache
        .async_set_cache_pipeline(entries, Some(30.0_f64))
        .await
        .expect("a pipeline write");
    assert_eq!(
        cache.operations.state().set_ttls,
        vec![
            ("a".to_owned(), Some(30.0_f64)),
            ("b".to_owned(), Some(30.0_f64))
        ]
    );
    assert_eq!(
        cache.async_get_ttl("a").await.expect("a ttl"),
        Some(30.0_f64)
    );
}

fn counter_increments() -> Vec<RedisPipelineIncrement> {
    vec![
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
    ]
}

#[tokio::test]
async fn memory_increment_pipeline_returns_each_result() {
    let cache = cache(600);
    assert_eq!(
        cache
            .async_increment_pipeline(counter_increments())
            .await
            .expect("a pipeline"),
        vec![1.5_f64, 3.5_f64, 1.0_f64]
    );
    assert_eq!(
        cache.async_get_cache("a").await.expect("a read"),
        Some(CacheValue::Float(3.5_f64))
    );
    // LiteLLM's pipeline hands each record to async_increment without its ttl, so the default applies
    assert!(
        cache
            .operations
            .state()
            .increments
            .iter()
            .all(|(_, _, ttl)| ttl.is_none())
    );
    assert_eq!(
        cache.async_get_ttl("a").await.expect("a ttl"),
        Some(600.0_f64)
    );
}

#[tokio::test]
async fn memory_increment_forwards_the_value_and_the_ttl() {
    let cache = cache(600);
    let result = cache
        .async_increment("counter", 1.5_f64, Some(5.0_f64))
        .await
        .expect("an increment");
    assert_eq!(Some(result), Some(1.5_f64));
    assert_eq!(
        cache.async_get_ttl("counter").await.expect("a ttl"),
        Some(5.0_f64)
    );
}

#[tokio::test]
async fn memory_increment_rejects_a_non_number() {
    let cache = cache(600);
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
async fn memory_get_ttl_returns_the_stored_entry() {
    let cache = cache(600);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(1030.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(1030.0_f64)
    );
    assert_eq!(cache.async_get_ttl("missing").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_oldest_keys_orders_by_expiry() {
    let cache = cache(600);
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
async fn memory_oldest_keys_keeps_insertion_order_for_equal_expiries() {
    let cache = cache(600);
    cache
        .async_set_cache("b", CacheValue::Integer(1), Some(20.0_f64))
        .await
        .expect("a stored value");
    cache
        .async_set_cache("a", CacheValue::Integer(1), Some(20.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache
            .async_get_oldest_n_keys(2)
            .await
            .expect("the oldest keys"),
        vec!["b".to_owned(), "a".to_owned()]
    );
}

#[tokio::test]
async fn memory_delete_removes_the_value_and_the_ttl() {
    let cache = cache(600);
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
    let cache = cache(600);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    cache.flush_cache().expect("a flush");
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key").await.expect("a ttl"), None);
}

#[tokio::test]
async fn memory_disconnect_succeeds() {
    cache(600).disconnect().await.expect("a disconnect");
}

#[tokio::test]
async fn memory_connection_test_is_unsupported() {
    let error = cache(600).test_connection().await.expect_err("unsupported");
    assert!(matches!(
        error,
        BackendError::Unsupported("test_connection")
    ));
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

type SharedClock = Arc<Mutex<f64>>;

fn local(start: f64) -> (MemoryCache<LocalMemoryOps>, SharedClock) {
    let now = Arc::new(Mutex::new(start));
    let reader = Arc::clone(&now);
    let ops = LocalMemoryOps::new(
        1024,
        Arc::new(move || *reader.lock().unwrap_or_else(PoisonError::into_inner)),
    )
    .expect("a local memory cache");
    (MemoryCache { operations: ops }, now)
}

fn advance(now: &SharedClock, to: f64) {
    *now.lock().unwrap_or_else(PoisonError::into_inner) = to;
}

#[tokio::test]
async fn local_memory_defaults_to_six_hundred_seconds() {
    let (cache, _now) = local(1000.0_f64);
    assert_eq!(cache.default_ttl(), 600);
    cache
        .async_set_cache("key", CacheValue::Integer(1), None)
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(1600.0_f64)
    );
}

#[tokio::test]
async fn local_memory_get_ttl_is_the_absolute_expiry() {
    let (cache, now) = local(1000.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(30.0_f64))
        .await
        .expect("a stored value");
    advance(&now, 1010.0_f64);
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(1030.0_f64)
    );
    assert_eq!(cache.async_get_ttl("missing").await.expect("a ttl"), None);
}

#[tokio::test]
async fn local_memory_get_ttl_reports_an_expired_key_until_a_read_drops_it() {
    let (cache, now) = local(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(10.0_f64))
        .await
        .expect("a stored value");
    advance(&now, 100.0_f64);
    assert_eq!(
        cache.async_get_ttl("key").await.expect("a ttl"),
        Some(10.0_f64)
    );
    assert_eq!(
        cache
            .async_get_oldest_n_keys(5)
            .await
            .expect("the oldest keys"),
        vec!["key".to_owned()]
    );
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
    assert_eq!(cache.async_get_ttl("key").await.expect("a ttl"), None);
}

#[tokio::test]
async fn local_memory_expires_a_key_only_after_its_expiry() {
    let (cache, now) = local(0.0_f64);
    cache
        .async_set_cache("key", CacheValue::Integer(1), Some(10.0_f64))
        .await
        .expect("a stored value");
    advance(&now, 10.0_f64);
    assert_eq!(
        cache.async_get_cache("key").await.expect("a read"),
        Some(CacheValue::Integer(1))
    );
    advance(&now, 10.5_f64);
    assert_eq!(cache.async_get_cache("key").await.expect("a read"), None);
}

#[tokio::test]
async fn local_memory_oldest_keys_orders_by_expiry_then_by_key() {
    let (cache, _now) = local(0.0_f64);
    let equal = vec![
        ("d".to_owned(), CacheValue::Integer(1)),
        ("b".to_owned(), CacheValue::Integer(1)),
        ("a".to_owned(), CacheValue::Integer(1)),
    ];
    cache
        .async_set_cache_pipeline(equal, Some(20.0_f64))
        .await
        .expect("a pipeline write");
    cache
        .async_set_cache("c", CacheValue::Integer(1), Some(5.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache
            .async_get_oldest_n_keys(3)
            .await
            .expect("the oldest keys"),
        vec!["c".to_owned(), "a".to_owned(), "b".to_owned()]
    );
}

async fn fill_local(cache: &MemoryCache<LocalMemoryOps>, count: u32) {
    for index in 0..count {
        cache
            .async_set_cache(
                &format!("key{index:03}"),
                CacheValue::Integer(1),
                Some(f64::from(index.saturating_add(100_u32))),
            )
            .await
            .expect("a stored value");
    }
}

#[tokio::test]
async fn local_memory_holds_two_hundred_keys_and_evicts_the_earliest_expiry_after_that() {
    let (cache, _now) = local(0.0_f64);
    fill_local(&cache, 200).await;
    assert!(
        cache
            .async_get_cache("key000")
            .await
            .expect("a read")
            .is_some()
    );
    assert_eq!(
        cache
            .async_get_oldest_n_keys(500)
            .await
            .expect("the oldest keys")
            .len(),
        200
    );
    cache
        .async_set_cache("overflow", CacheValue::Integer(1), Some(600.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(cache.async_get_cache("key000").await.expect("a read"), None);
    assert!(
        cache
            .async_get_cache("key001")
            .await
            .expect("a read")
            .is_some()
    );
    assert!(
        cache
            .async_get_cache("overflow")
            .await
            .expect("a read")
            .is_some()
    );
}

#[tokio::test]
async fn local_memory_increment_adds_to_a_stored_number_and_keeps_its_expiry() {
    let (cache, now) = local(0.0_f64);
    cache
        .async_set_cache("count", CacheValue::Integer(2), Some(5.0_f64))
        .await
        .expect("a stored value");
    let sum = cache
        .async_increment("count", 1.5_f64, Some(50.0_f64))
        .await
        .expect("an increment");
    assert_eq!(Some(sum), Some(3.5_f64));
    assert_eq!(
        cache.async_get_ttl("count").await.expect("a ttl"),
        Some(5.0_f64)
    );
    advance(&now, 10.0_f64);
    let restarted = cache
        .async_increment("count", 1.0_f64, Some(50.0_f64))
        .await
        .expect("an increment");
    assert_eq!(Some(restarted), Some(1.0_f64));
    assert_eq!(
        cache.async_get_ttl("count").await.expect("a ttl"),
        Some(60.0_f64)
    );
}

#[tokio::test]
async fn local_memory_increment_pipeline_ignores_the_record_ttl() {
    let (cache, _now) = local(100.0_f64);
    assert_eq!(
        cache
            .async_increment_pipeline(counter_increments())
            .await
            .expect("a pipeline"),
        vec![1.5_f64, 3.5_f64, 1.0_f64]
    );
    assert_eq!(
        cache.async_get_ttl("a").await.expect("a ttl"),
        Some(700.0_f64)
    );
}

#[tokio::test]
async fn local_memory_increment_rejects_a_non_number_and_a_non_finite_sum() {
    let (cache, _now) = local(0.0_f64);
    cache
        .async_set_cache("text", CacheValue::Text("words".to_owned()), None)
        .await
        .expect("a stored value");
    let error = cache
        .async_increment("text", 1.0_f64, None)
        .await
        .expect_err("a non-number");
    assert!(matches!(error, BackendError::NotNumeric { key } if key == "text"));
    cache
        .async_set_cache("big", CacheValue::Float(f64::MAX), None)
        .await
        .expect("a stored value");
    let overflow = cache
        .async_increment("big", f64::MAX, None)
        .await
        .expect_err("a non-finite sum");
    assert!(matches!(overflow, BackendError::IncrementOutOfRange { key } if key == "big"));
    assert_eq!(
        cache.async_get_cache("big").await.expect("a read"),
        Some(CacheValue::Float(f64::MAX))
    );
}

#[tokio::test]
async fn local_memory_batch_get_delete_and_flush() {
    let (cache, _now) = local(0.0_f64);
    let entries = vec![
        ("a".to_owned(), CacheValue::Integer(1)),
        ("b".to_owned(), CacheValue::Integer(2)),
    ];
    cache
        .async_set_cache_pipeline(entries, None)
        .await
        .expect("a pipeline write");
    let keys = ["a".to_owned(), "missing".to_owned(), "b".to_owned()];
    assert_eq!(
        cache.async_batch_get_cache(&keys).await.expect("a batch"),
        vec![
            Some(CacheValue::Integer(1)),
            None,
            Some(CacheValue::Integer(2))
        ]
    );
    cache.delete_cache("a").expect("a delete");
    assert_eq!(cache.async_get_ttl("a").await.expect("a ttl"), None);
    cache.flush_cache().expect("a flush");
    assert_eq!(cache.async_get_cache("b").await.expect("a read"), None);
    assert_eq!(
        cache
            .async_get_oldest_n_keys(5)
            .await
            .expect("the oldest keys"),
        Vec::<String>::new()
    );
    cache
        .async_set_cache("b", CacheValue::Integer(3), Some(7.0_f64))
        .await
        .expect("a stored value");
    assert_eq!(
        cache.async_get_ttl("b").await.expect("a ttl"),
        Some(7.0_f64)
    );
}
