use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::request_batch::scope::request_redis_batch_scope;

use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::ActiveSpendBatch;
use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::error::{LayersError, SpendWriteError};
use crate::spend_writes::layers::CounterLayers;

use super::refusing::refusing_redis_cache;
use super::support::{Failure, Fault, KEY, Logs, decimal, dual_cache, fixture};

#[tokio::test]
async fn a_batched_amount_is_warm_and_lands_in_memory() {
    let fx = fixture(true);
    fx.batch.answer_with(CounterValue::Amount(decimal("1.5")));
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("the batched counter answers");
    assert!(warm);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("1.5")))
    );
    assert!(!fx.journal.records(&format!("redis_get:{KEY}")));
    assert!(!fx.journal.records(&format!("memory_get:{KEY}")));
}

#[tokio::test]
async fn a_batched_miss_is_cold_even_when_memory_holds_a_value() {
    let fx = fixture(true);
    fx.batch.answer_with(CounterValue::Missing);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("9")));
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("the batched miss answers");
    assert!(!warm);
    assert!(!fx.journal.records(&format!("redis_get:{KEY}")));
}

#[tokio::test]
async fn a_redis_value_is_warm_and_lands_in_memory() {
    let fx = fixture(true);
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("2")));
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("the redis counter answers");
    assert!(warm);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("2")))
    );
}

#[tokio::test]
async fn a_redis_read_failure_falls_back_to_memory() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fx.layers.faults.add(Fault::RedisGet);
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("the memory tier answers after the redis failure");
    assert!(warm);
    assert!(logs.contains("falling back to in-memory"));
}

#[tokio::test]
async fn a_memory_value_alone_is_warm_without_redis() {
    let fx = fixture(false);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("4")));
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("the memory counter answers");
    assert!(warm);
    assert!(!fx.journal.records(&format!("redis_get:{KEY}")));
}

#[tokio::test]
async fn nothing_anywhere_is_cold() {
    let fx = fixture(true);
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("an empty counter is cold");
    assert!(!warm);
    assert!(fx.journal.records(&format!("redis_get:{KEY}")));
    assert!(!fx.journal.records(&format!("memory_get:{KEY}")));
}

#[tokio::test]
async fn a_clean_redis_miss_is_cold_even_when_memory_holds_a_value() {
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("9")));
    let warm = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect("the redis miss answers");
    assert!(!warm);
    assert!(fx.journal.records(&format!("redis_get:{KEY}")));
    assert!(!fx.journal.records(&format!("memory_get:{KEY}")));
}

#[tokio::test]
async fn a_batch_read_failure_propagates() {
    let fx = fixture(false);
    fx.batch.fail();
    let error = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect_err("the batch failure surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Batch(SpendCounterBatchError::Cancelled)
    ));
}

#[tokio::test]
async fn a_memory_read_failure_propagates() {
    let fx = fixture(false);
    fx.layers.faults.add(Fault::MemoryGet);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let error = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect_err("the memory failure surfaces");
    assert!(matches!(error, SpendWriteError::Cache(_)));
}

#[tokio::test]
async fn a_memory_write_failure_propagates_on_a_batch_hit() {
    let fx = fixture(false);
    fx.batch.answer_with(CounterValue::Amount(decimal("1")));
    fx.layers.faults.add(Fault::SetMemoryValue);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let error = fx
        .writes
        .is_spend_counter_cache_warm(KEY)
        .await
        .expect_err("the memory write failure surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Memory(_))
    ));
}

#[tokio::test]
async fn the_counter_scope_answers_a_recorded_value() {
    let redis = refusing_redis_cache().await;
    let scope = SpendCounterScope::default();
    let keys = BTreeSet::from([KEY.to_owned()]);
    let counted: &dyn CounterBatch = &scope;
    let (_batches, read) = request_redis_batch_scope(Duration::from_secs(1), async {
        let _guard = scope.enter(Some(&redis), &keys);
        counted.record(KEY, decimal("5"));
        counted.read(KEY).await.expect("the recorded counter reads")
    })
    .await;
    assert_eq!(read, Some(CounterValue::Amount(decimal("5"))));
}

#[tokio::test]
async fn the_counter_scope_forgets_a_recorded_value() {
    let redis = refusing_redis_cache().await;
    let scope = SpendCounterScope::default();
    let keys = BTreeSet::from([KEY.to_owned()]);
    let counted: &dyn CounterBatch = &scope;
    let (_batches, read) = request_redis_batch_scope(Duration::from_secs(1), async {
        let _guard = scope.enter(Some(&redis), &keys);
        counted.record(KEY, decimal("5"));
        counted.forget(KEY);
        scope
            .read_spend_counter(KEY)
            .await
            .expect("the forgotten counter answers")
    })
    .await;
    assert_eq!(read, None);
}

#[tokio::test]
async fn the_real_memory_tier_stores_the_counter_value() {
    let cache = dual_cache(None);
    cache
        .set_memory_value(KEY, ReseedCacheValue::Number(decimal("2.5")))
        .await
        .expect("the memory tier stores the counter");
    let stored = cache
        .in_memory_cache
        .get_cache(KEY)
        .expect("the memory tier reads");
    assert_eq!(stored, Some(CacheValue::Float(2.5)));
    cache
        .set_memory_value(KEY, ReseedCacheValue::Unreadable)
        .await
        .expect("an unreadable value stores nothing");
    let stored = cache
        .in_memory_cache
        .get_cache(KEY)
        .expect("the memory tier reads");
    assert_eq!(stored, Some(CacheValue::Float(2.5)));
}

#[tokio::test]
async fn the_batch_adapter_reads_and_records_through_the_counter_batch() {
    let fx = fixture(false);
    fx.batch.answer_with(CounterValue::Amount(decimal("3")));
    let adapter: Arc<dyn CounterBatch> = Arc::clone(&fx.batch);
    let read = ActiveSpendBatch::read(&adapter, KEY)
        .await
        .expect("the adapter reads");
    ActiveSpendBatch::record(&adapter, KEY, decimal("3"));
    assert_eq!(read, Some(CounterValue::Amount(decimal("3"))));
    assert_eq!(fx.batch.recorded(), vec![(KEY.to_owned(), decimal("3"))]);
}

#[tokio::test]
async fn a_scope_without_a_redis_tier_answers_nothing() {
    let scope = SpendCounterScope::default();
    let keys = BTreeSet::from([KEY.to_owned()]);
    let counted: &dyn CounterBatch = &scope;
    let read = {
        let _guard = scope.enter(None, &keys);
        counted.record(KEY, decimal("5"));
        counted.read(KEY).await.expect("the empty scope answers")
    };
    assert_eq!(read, None);
}
