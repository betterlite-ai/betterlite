#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_models::decisions::Json;
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
use betterlite_redis::request_batch::scope::request_redis_batch_scope;

use super::fixtures;
use super::support::{auth, json};
use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_prefetch::fill::{PrefetchClock, SystemPrefetchClock};
use crate::auth_prefetch::memory::{PrefetchCache, PrefetchMemory};
use crate::auth_prefetch::orchestrator::AdmissionCounterBinding;
use crate::auth_prefetch::permission::lookup::ObjectPermissionCache;
use crate::auth_prefetch::redis_rows::{PrefetchRedis, RedisRows, RedisRowsError};
use crate::spend_counter_batch::scope::SpendCounterScope;

const CLOCK_SECONDS: f64 = 1000.0;
const MANAGEMENT_TTL: f64 = 600.0;
const FALLBACK_TTL: f64 = 45.0;
const DEAD_REDIS: &str = r#"{"host":"127.0.0.1","port":1}"#;

struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

#[derive(Default)]
struct RecordingGate {
    calls: Mutex<Vec<&'static str>>,
}

impl RecordingGate {
    fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BreakerGate for RecordingGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| CLOCK_SECONDS);
    Arc::new(LocalMemoryOps::new(1024, clock).expect("the local memory tier builds"))
}

fn real_cache(default_in_memory_ttl: Option<f64>) -> AuthCache<DualCache<LocalMemoryOps>> {
    let settings = DualCacheSettings {
        default_in_memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    gateway_cache(memory(), memory(), None, settings, FALLBACK_TTL)
}

async fn dead_redis(gate: Arc<dyn BreakerGate>) -> RedisCache {
    let overrides: RedisOverrides =
        serde_json::from_str(DEAD_REDIS).expect("the override fixture builds");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate,
    };
    connect(options, &BTreeMap::<String, String>::new())
        .await
        .expect("the dead client builds")
}

fn permission_model(permission_id: &str) -> LiteLLMObjectPermissionTable {
    let row = fixtures::permission_row(permission_id);
    let wire = serde_json::to_value(row).expect("the row serializes");
    serde_json::from_value(wire).expect("the permission model validates")
}

#[test]
fn real_cache_management_and_default_ttls() {
    let cache = real_cache(Some(MANAGEMENT_TTL));
    assert_eq!(PrefetchCache::management_ttl(&cache), MANAGEMENT_TTL);
    assert_eq!(
        PrefetchCache::default_memory_ttl(&cache),
        Some(MANAGEMENT_TTL)
    );
    let fallback_cache = real_cache(None);
    assert_eq!(PrefetchCache::management_ttl(&fallback_cache), FALLBACK_TTL);
    assert_eq!(PrefetchCache::default_memory_ttl(&fallback_cache), None);
}

#[test]
fn real_cache_memory_roundtrip() {
    let cache = real_cache(Some(MANAGEMENT_TTL));
    let memory = PrefetchCache::ordinary_memory(&cache);
    assert!(memory.get("prefetch-key").is_none());
    memory
        .set(
            "prefetch-key",
            CacheValue::Text("row".to_owned()),
            Some(30.0),
        )
        .expect("the memory write");
    assert_eq!(
        memory.get("prefetch-key"),
        Some(CacheValue::Text("row".to_owned()))
    );
}

#[tokio::test]
async fn real_cache_partitions_and_redis() {
    let cache = real_cache(Some(MANAGEMENT_TTL));
    let hex = "c".repeat(64);
    PrefetchCache::memory_for(&cache, &hex)
        .set(&hex, CacheValue::Text("key".to_owned()), Some(5.0))
        .expect("the key partition write");
    assert_eq!(
        PrefetchCache::memory_for(&cache, &hex).get(&hex),
        Some(CacheValue::Text("key".to_owned()))
    );
    assert!(PrefetchCache::ordinary_memory(&cache).get(&hex).is_none());
    assert!(PrefetchCache::redis(&cache).is_none());

    let settings = DualCacheSettings {
        default_in_memory_ttl: Some(MANAGEMENT_TTL),
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    let attached = gateway_cache(
        memory(),
        memory(),
        Some(Arc::new(dead_redis(Arc::new(OpenGate)).await)),
        settings,
        FALLBACK_TTL,
    );
    assert!(PrefetchCache::redis(&attached).is_some());
}

#[tokio::test]
async fn real_cache_permission_roundtrip() {
    let cache = real_cache(Some(MANAGEMENT_TTL));
    let permission = permission_model("perm-1");
    ObjectPermissionCache::set(&cache, "object_permission_id:perm-1", &permission, 90.0)
        .await
        .expect("the permission write");
    let cached = ObjectPermissionCache::get(&cache, "object_permission_id:perm-1")
        .await
        .expect("the permission read");
    assert_eq!(cached, Some(permission));
    assert_eq!(
        ObjectPermissionCache::management_ttl(&cache),
        MANAGEMENT_TTL
    );
}

#[tokio::test]
async fn redis_rows_direct_read_and_guards() {
    let client = Arc::new(dead_redis(Arc::new(OpenGate)).await);
    let rows = RedisRows { client };
    assert!(!rows.has_batch());
    let read = rows
        .read(&["user-1".to_owned()])
        .await
        .expect("the dead direct read answers");
    let Json::Object(fields) = read else {
        panic!("the read map is an object");
    };
    assert!(fields.is_empty());
    assert!(matches!(
        rows.queue_set("k", &Json::Text("v".to_owned()), Some(1.0)),
        Err(RedisRowsError::NoBatch)
    ));
    let outcome = rows
        .set_pipeline(&[("k".to_owned(), Json::Text("v".to_owned()), Some(1e30_f64))])
        .await;
    assert!(matches!(outcome, Err(RedisRowsError::Write { .. })));
}

#[tokio::test]
async fn redis_rows_batch_scope_detection() {
    let client = Arc::new(dead_redis(Arc::new(OpenGate)).await);
    let rows = RedisRows { client };
    let (_batches, ()) = request_redis_batch_scope(Duration::from_millis(5), async {
        assert!(rows.has_batch());
    })
    .await;
}

#[test]
fn system_clock_reads_unix_seconds() {
    let now = SystemPrefetchClock.now();
    assert!(now > 1_700_000_000.0, "the clock reads unix seconds: {now}");
}

#[tokio::test]
async fn spend_scope_trait_binds_admission_counters() {
    let gate = Arc::new(RecordingGate::default());
    let recording: Arc<dyn BreakerGate> = Arc::clone(&gate);
    let cache = Arc::new(dead_redis(recording).await);
    let scope = SpendCounterScope::default();
    let _guard = scope.enter(Some(&cache), &BTreeSet::new());
    let api_key = auth(&[("user_id", json(r#""user-1""#))]);
    <SpendCounterScope as AdmissionCounterBinding>::bind_admission_counter_keys(
        &scope, &api_key, None,
    );
    let value = scope
        .read_spend_counter("spend:user:user-1")
        .await
        .expect("the batched read");
    assert!(value.is_none());
    assert_eq!(gate.calls(), vec!["async_batch_get_cache"]);
}
