use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::sync::Arc;

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
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;

use super::support::{Captured, Fixture, KEY, decimal};
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reads::SpendReads;
use crate::spend_reads::batch::SpendBatch;
use crate::spend_reseed::cache::ActiveSpendBatch;
use crate::spend_reseed::cache::ReseedCache as _;
use crate::spend_reseed::values::ReseedCacheValue;

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

// refuses every command before a client is opened, so no test ever waits on a socket
struct RefuseGate;

impl BreakerGate for RefuseGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

fn dual_settings() -> DualCacheSettings {
    DualCacheSettings::from_environment(&BTreeMap::new())
}

fn unreachable_overrides() -> RedisOverrides {
    let mut overrides: RedisOverrides =
        serde_json::from_str("{}").expect("the empty overrides deserialize");
    overrides.host = Some("127.0.0.1".to_owned());
    overrides.port = Some(IntOrText::Value(1_i64));
    overrides
}

async fn refusing_redis() -> Arc<RedisCache> {
    let options = RedisCacheOptions {
        overrides: unreachable_overrides(),
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60_i64,
        redis_flush_size: None,
        default_major_version: 7_u32,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(RefuseGate),
    };
    let environment = BTreeMap::<String, String>::new();
    Arc::new(
        connect(options, &environment)
            .await
            .expect("the cache opens without a server"),
    )
}

#[tokio::test]
async fn the_scope_adapter_records_reads_and_forgets_its_active_batch() {
    let redis = refusing_redis().await;
    let scope = SpendCounterScope::default();
    let _guard = scope.enter(Some(&redis), &BTreeSet::from([KEY.to_owned()]));

    ActiveSpendBatch::record(&scope, KEY, decimal("4"));

    assert_eq!(
        ActiveSpendBatch::read(&scope, KEY)
            .await
            .expect("the adapter read"),
        Some(CounterValue::Amount(decimal("4")))
    );
    SpendBatch::forget(&scope, KEY);
    assert_eq!(
        ActiveSpendBatch::read(&scope, KEY)
            .await
            .expect("the read after the forget"),
        None
    );
}

#[tokio::test]
async fn a_refused_redis_raise_keeps_the_memory_repair_and_reports_the_failure() {
    let logs = Captured::default();
    let _guard = logs.install();
    let memory = Arc::new(
        LocalMemoryOps::new(1_024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"),
    );
    let cache = DualCache::new(memory, Some(refusing_redis().await), dual_settings());
    let fixture = Fixture::new(false);
    let reads = SpendReads {
        reseed: &fixture.reseed,
        cache: &cache,
        batch: Some(fixture.batch.as_ref()),
        fail_closed_budget_enforcement: false,
    };

    reads.repair_stale_spend_counter(KEY, decimal("5")).await;

    assert_eq!(
        cache.memory_get(KEY).await.expect("the memory read"),
        Some(ReseedCacheValue::Number(decimal("5")))
    );
    assert!(fixture.batch.recorded().is_empty());
    assert!(logs.has("Unable to repair stale spend counter in Redis"));
}
