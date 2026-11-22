use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_context::local_ttl_cache::CacheValue;

use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::error::{LayersError, SpendWriteError};
use crate::spend_writes::layers::CounterLayers;

use super::support::{
    Failure, FakeBatch, Fault, Journal, KEY, decimal, dual_cache, fixture, refusing_redis_cache,
    writes_over,
};

#[tokio::test]
async fn increments_through_redis_and_records_the_new_value() {
    let fx = fixture(true);
    let counted = fx
        .writes
        .increment_spend_counter(KEY, decimal("2.5"))
        .await
        .expect("the redis increment answers");
    assert_eq!(counted, Some(decimal("2.5")));
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("2.5")))
    );
    assert_eq!(fx.batch.recorded(), vec![(KEY.to_owned(), decimal("2.5"))]);
    let expected = vec![
        format!("increment_redis:{KEY}"),
        format!("memory_set:{KEY}"),
        format!("batch_record:{KEY}:2.5"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn increments_in_memory_without_a_redis_tier() {
    let fx = fixture(false);
    let counted = fx
        .writes
        .increment_spend_counter(KEY, decimal("9"))
        .await
        .expect("the memory increment answers");
    assert_eq!(counted, Some(decimal("9")));
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("9")));
    assert_eq!(fx.journal.count("increment_redis:"), 0);
    assert!(fx.batch.recorded().is_empty());
}

#[tokio::test]
async fn a_failed_redis_increment_invalidates_before_returning() {
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("1")));
    fx.layers.faults.add(Fault::IncrementRedis);
    let error = fx
        .writes
        .increment_spend_counter(KEY, decimal("2"))
        .await
        .expect_err("the refused increment surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Redis(_))
    ));
    assert!(error.is_transient());
    assert_eq!(fx.layers.memory_value(KEY), None);
    assert_eq!(fx.layers.redis_value(), None);
    let expected = vec![
        format!("increment_redis:{KEY}"),
        format!("batch_forget:{KEY}"),
        format!("delete_memory:{KEY}"),
        format!("delete_redis:{KEY}"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn a_memory_write_failure_skips_the_record_and_the_invalidation() {
    let fx = fixture(true);
    fx.layers.faults.add(Fault::MemorySet);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let error = fx
        .writes
        .increment_spend_counter(KEY, decimal("2"))
        .await
        .expect_err("the memory write failure surfaces");
    assert!(matches!(error, SpendWriteError::Cache(_)));
    assert!(fx.batch.recorded().is_empty());
    assert_eq!(fx.journal.count("batch_forget:"), 0);
}

#[tokio::test]
async fn a_negative_increment_reaches_the_redis_counter() {
    let fx = fixture(true);
    let counted = fx
        .writes
        .increment_spend_counter(KEY, decimal("-2.5"))
        .await
        .expect("the negative increment answers");
    assert_eq!(counted, Some(decimal("-2.5")));
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("-2.5")));
}

#[tokio::test]
async fn the_real_cache_increments_in_memory_without_a_redis_tier() {
    let layers = Arc::new(dual_cache(None));
    let batch = Arc::new(FakeBatch::new(Journal::default()));
    let writes = writes_over(Arc::clone(&layers), Arc::clone(&batch));
    let counted = writes
        .increment_spend_counter(KEY, decimal("4"))
        .await
        .expect("the memory tier increments");
    assert_eq!(counted, Some(decimal("4")));
    let stored = layers
        .in_memory_cache
        .get_cache(KEY)
        .expect("the memory tier reads");
    assert_eq!(stored, Some(CacheValue::Float(4.0)));
}

#[tokio::test]
async fn a_refused_redis_increment_is_transient_and_clears_the_counter() {
    let redis = refusing_redis_cache().await;
    let layers = Arc::new(dual_cache(Some(redis)));
    layers
        .in_memory_cache
        .set_cache(KEY, CacheValue::Float(9.0), None)
        .expect("the memory tier stores the fixture");
    let batch = Arc::new(FakeBatch::new(Journal::default()));
    let writes = writes_over(Arc::clone(&layers), Arc::clone(&batch));
    let error = writes
        .increment_spend_counter(KEY, decimal("1"))
        .await
        .expect_err("the breaker refuses the increment");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Redis(_))
    ));
    assert!(error.is_transient());
    let stored = layers
        .in_memory_cache
        .get_cache(KEY)
        .expect("the memory tier reads");
    assert_eq!(stored, None);
    assert!(batch.journal.records(&format!("batch_forget:{KEY}")));
}

#[tokio::test]
async fn the_real_redis_increment_requires_a_tier() {
    let cache = dual_cache(None);
    let error = cache
        .increment_redis(KEY, decimal("1"))
        .await
        .expect_err("the missing tier answers");
    assert!(matches!(error, LayersError::Unavailable));
    assert!(!error.is_transient());
}
