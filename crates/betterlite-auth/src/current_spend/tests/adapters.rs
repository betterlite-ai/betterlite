use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use rust_decimal::Decimal;

use super::support::money;
use crate::current_spend::cache::{CounterBatch as _, CounterCache as _};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::{ActiveSpendBatch as _, ReseedCache as _};
use crate::spend_reseed::values::ReseedCacheValue;

fn local_cache(clock: &Arc<Mutex<f64>>) -> DualCache<LocalMemoryOps> {
    let reading = Arc::clone(clock);
    let memory = LocalMemoryOps::new(
        1024,
        Arc::new(move || *reading.lock().unwrap_or_else(PoisonError::into_inner)),
    )
    .expect("the local memory tier builds");
    let settings = DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1_000,
        redis_delete_chunk_size: 1_000,
    };
    DualCache::new(Arc::new(memory), None, settings)
}

#[tokio::test]
async fn dual_cache_marker_expires_after_its_ttl_and_skips_redis() {
    let clock = Arc::new(Mutex::new(0.0_f64));
    let cache = local_cache(&clock);
    cache
        .memory_set_expiring("spend_db_floor:spend:key:k", money("2.5"), 5)
        .unwrap();
    *clock.lock().unwrap() = 4.9_f64;
    let held = cache
        .memory_get("spend_db_floor:spend:key:k")
        .await
        .unwrap();
    assert_eq!(held, Some(ReseedCacheValue::Number(money("2.5"))));
    *clock.lock().unwrap() = 5.1_f64;
    let expired = cache
        .memory_get("spend_db_floor:spend:key:k")
        .await
        .unwrap();
    assert_eq!(expired, None);
    let repaired = cache
        .redis_set_max("spend:key:k", money("2.5"))
        .await
        .unwrap();
    assert_eq!(repaired, None);
}

#[tokio::test]
async fn scope_batch_answers_nothing_without_an_active_batch() {
    let scope = SpendCounterScope::default();
    scope.record("spend:key:k", Decimal::ONE);
    scope.forget("spend:key:k");
    assert_eq!(scope.read("spend:key:k").await.unwrap(), None);
}
