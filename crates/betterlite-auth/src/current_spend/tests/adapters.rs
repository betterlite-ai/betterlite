use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use rust_decimal::Decimal;

use super::support::money;
use crate::auth_invalidation::SPEND_DB_FLOOR_TTL_SECONDS;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reads::batch::SpendBatch as _;
use crate::spend_reads::cache::SpendReadCache as _;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ActiveSpendBatch as _, ReseedCache as _};
use crate::spend_reseed::values::ReseedCacheValue;

// refuses every Redis call at the breaker, so nothing ever dials the dead address
struct Refused;

impl ServiceHooks for Refused {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

impl BreakerGate for Refused {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn local_cache(
    clock: &Arc<Mutex<f64>>,
    redis: Option<Arc<RedisCache>>,
) -> DualCache<LocalMemoryOps> {
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
    DualCache::new(Arc::new(memory), redis, settings)
}

#[tokio::test]
async fn dual_cache_marker_expires_after_its_ttl_and_skips_redis() {
    let clock = Arc::new(Mutex::new(0.0_f64));
    let cache = local_cache(&clock, None);
    cache
        .memory_set_ttl(
            "spend_db_floor:spend:key:k",
            money("2.5"),
            Some(SPEND_DB_FLOOR_TTL_SECONDS),
        )
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
async fn refused_redis_fails_set_max_and_the_open_batch_records_and_forgets() {
    let overrides: RedisOverrides = serde_json::from_str(r#"{"host": "127.0.0.1", "port": 1}"#)
        .expect("the dead Redis address parses");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(Refused),
        gate: Arc::new(Refused),
    };
    let env = BTreeMap::<String, String>::new();
    let redis = Arc::new(
        connect(options, &env)
            .await
            .expect("a refused Redis still builds a cache"),
    );
    let clock = Arc::new(Mutex::new(0.0_f64));
    let cache = local_cache(&clock, Some(Arc::clone(&redis)));
    let refused = cache.redis_set_max("spend:key:k", money("2.5")).await;
    assert!(
        matches!(refused, Err(ReseedCacheError::Redis(_))),
        "{refused:?}"
    );
    let scope = SpendCounterScope::default();
    let _open = scope.enter(Some(&redis), &BTreeSet::new());
    scope.record("spend:key:k", money("7.5"));
    let recorded = scope.read("spend:key:k").await.unwrap();
    assert_eq!(recorded, Some(CounterValue::Amount(money("7.5"))));
    scope.forget("spend:key:k");
    assert_eq!(scope.read("spend:key:k").await.unwrap(), None);
}

#[tokio::test]
async fn scope_batch_answers_nothing_without_an_active_batch() {
    let scope = SpendCounterScope::default();
    scope.record("spend:key:k", Decimal::ONE);
    scope.forget("spend:key:k");
    assert_eq!(scope.read("spend:key:k").await.unwrap(), None);
}
