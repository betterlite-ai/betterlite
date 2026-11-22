use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_context::local_ttl_cache::CacheValue;

use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::error::LayersError;
use crate::spend_writes::invalidation::invalidate_spend_counter;
use crate::spend_writes::layers::CounterLayers;

use super::support::{
    Failure, Fault, KEY, Logs, decimal, dual_cache, fixture, refusing_redis_cache,
};

#[tokio::test]
async fn invalidation_clears_the_batch_memory_and_redis() {
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("1")));
    let counted: &dyn CounterBatch = &*fx.batch;
    invalidate_spend_counter(&*fx.layers, counted, KEY).await;
    let expected = vec![
        format!("batch_forget:{KEY}"),
        format!("delete_memory:{KEY}"),
        format!("delete_redis:{KEY}"),
    ];
    assert_eq!(fx.journal.events(), expected);
    assert_eq!(fx.layers.memory_value(KEY), None);
    assert_eq!(fx.layers.redis_value(), None);
}

#[tokio::test]
async fn a_memory_delete_failure_still_clears_redis() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("1")));
    fx.layers.faults.add(Fault::DeleteMemory);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Memory;
    let counted: &dyn CounterBatch = &*fx.batch;
    invalidate_spend_counter(&*fx.layers, counted, KEY).await;
    assert_eq!(fx.layers.redis_value(), None);
    assert_eq!(
        fx.layers.memory_value(KEY),
        Some(ReseedCacheValue::Number(decimal("1")))
    );
    assert!(logs.contains("Unable to delete the stale in-memory spend counter"));
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn a_redis_delete_failure_is_logged_at_debug() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers
        .put_memory(KEY, ReseedCacheValue::Number(decimal("1")));
    fx.layers.put_redis(ReseedCacheValue::Number(decimal("1")));
    fx.layers.faults.add(Fault::DeleteRedis);
    let counted: &dyn CounterBatch = &*fx.batch;
    invalidate_spend_counter(&*fx.layers, counted, KEY).await;
    assert_eq!(fx.layers.memory_value(KEY), None);
    assert_eq!(
        fx.layers.redis_value(),
        Some(ReseedCacheValue::Number(decimal("1")))
    );
    assert!(logs.contains("Unable to delete the stale spend counter after increment failure"));
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn no_redis_tier_skips_the_redis_delete() {
    let fx = fixture(false);
    fx.layers.faults.add(Fault::DeleteRedis);
    let counted: &dyn CounterBatch = &*fx.batch;
    invalidate_spend_counter(&*fx.layers, counted, KEY).await;
    assert_eq!(fx.journal.count("delete_redis:"), 0);
    let expected = vec![
        format!("batch_forget:{KEY}"),
        format!("delete_memory:{KEY}"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn the_real_memory_tier_deletes_its_value() {
    let cache = dual_cache(None);
    cache
        .in_memory_cache
        .set_cache(KEY, CacheValue::Float(1.0), None)
        .expect("the memory tier stores the fixture");
    cache
        .delete_memory(KEY)
        .await
        .expect("the memory tier deletes");
    let stored = cache
        .in_memory_cache
        .get_cache(KEY)
        .expect("the memory tier reads");
    assert_eq!(stored, None);
}

#[tokio::test]
async fn the_real_redis_delete_requires_a_tier() {
    let cache = dual_cache(None);
    let error = cache
        .delete_redis(KEY)
        .await
        .expect_err("the missing tier answers");
    assert!(matches!(error, LayersError::Unavailable));
}

#[tokio::test]
async fn the_real_redis_delete_reports_a_refused_delete() {
    let cache = dual_cache(Some(refusing_redis_cache().await));
    let error = cache
        .delete_redis(KEY)
        .await
        .expect_err("the breaker refuses the delete");
    assert!(matches!(error, LayersError::Redis(_)));
}
