#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::CacheBackend;
use betterlite_cache::configuration::cache::{Backend, Cache};
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::call_type::SUPPORTED_CALL_TYPES;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_admin::info_parse::{ClientListError, InfoValue};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use indexmap::IndexMap;

use crate::cache_routes::admin::{CacheAdminError, RedisAdmin};

pub(crate) struct FakeRedis {
    pub(crate) clients: Mutex<Result<Vec<IndexMap<String, String>>, ()>>,
    pub(crate) info: Mutex<Result<IndexMap<String, InfoValue>, ()>>,
    pub(crate) flush: Mutex<Result<(), ()>>,
    pub(crate) flushes: AtomicUsize,
}

pub(crate) fn fake_redis() -> FakeRedis {
    FakeRedis {
        clients: Mutex::new(Ok(Vec::new())),
        info: Mutex::new(Ok(IndexMap::new())),
        flush: Mutex::new(Ok(())),
        flushes: AtomicUsize::new(0),
    }
}

fn scripted<T>(cell: &Mutex<Result<T, ()>>) -> Result<T, ()>
where
    T: Clone,
{
    cell.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn redis_failure() -> CacheAdminError {
    CacheAdminError::ClientList {
        source: RedisAdminError::ClientList(ClientListError::OrphanToken),
    }
}

impl RedisAdmin for FakeRedis {
    fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, CacheAdminError> {
        scripted(&self.clients).map_err(|()| redis_failure())
    }

    fn info(&self) -> Result<IndexMap<String, InfoValue>, CacheAdminError> {
        scripted(&self.info).map_err(|()| CacheAdminError::Info {
            source: RedisAdminError::ClientList(ClientListError::OrphanToken),
        })
    }

    fn flushall(&self) -> Result<(), CacheAdminError> {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        let scripted = *self.flush.lock().unwrap_or_else(PoisonError::into_inner);
        scripted.map_err(|()| CacheAdminError::Flush {
            source: RedisAdminError::ClientList(ClientListError::OrphanToken),
        })
    }
}

// Panics the way a blocking Redis call would if its worker thread aborted.
pub(crate) struct PanickingRedis;

fn panicking<T>(value: Option<T>) -> T {
    value.expect("the blocking cache call panicked")
}

impl RedisAdmin for PanickingRedis {
    fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, CacheAdminError> {
        Ok(panicking(None::<Vec<IndexMap<String, String>>>))
    }

    fn info(&self) -> Result<IndexMap<String, InfoValue>, CacheAdminError> {
        Ok(panicking(None::<IndexMap<String, InfoValue>>))
    }

    fn flushall(&self) -> Result<(), CacheAdminError> {
        panicking(None::<()>);
        Ok(())
    }
}

pub(crate) struct StubBackend;

#[async_trait]
impl CacheBackend for StubBackend {
    fn default_ttl(&self) -> i64 {
        60_i64
    }
}

pub(crate) fn local_cache() -> Cache {
    Cache {
        backend: Backend::Injected(Arc::new(StubBackend)),
        cache_type: LiteLlmCacheType::Local,
        namespace: None,
        redis_flush_size: None,
        ttl: None,
        mode: CacheMode::DefaultOn,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
    }
}

pub(crate) struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

pub(crate) struct ClosedGate;

impl BreakerGate for ClosedGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

// The same overrides redis-py receives when nothing is configured.
pub(crate) fn overrides() -> RedisOverrides {
    serde_json::from_str("{}").expect("the empty overrides deserialize")
}

// A cache that opens without a server: every operation fails, which is the Redis-failure path the routes answer.
pub(crate) async fn unreachable_redis_cache() -> Cache {
    redis_cache_from(unreachable_overrides()).await
}

pub(crate) fn unreachable_overrides() -> RedisOverrides {
    let mut settings = overrides();
    settings.host = Some("127.0.0.1".to_owned());
    settings.port = Some(IntOrText::Value(1_i64));
    settings
}

pub(crate) async fn redis_cache_from(overrides: RedisOverrides) -> Cache {
    redis_cache_gated(overrides, Arc::new(ClosedGate)).await
}

// A cache whose breaker refuses every operation, so a write returns an error instead of being swallowed as a connection failure.
pub(crate) async fn breaker_open_redis_cache() -> Cache {
    redis_cache_gated(unreachable_overrides(), Arc::new(OpenGate)).await
}

async fn redis_cache_gated(overrides: RedisOverrides, gate: Arc<dyn BreakerGate>) -> Cache {
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60_i64,
        redis_flush_size: None,
        default_major_version: 7_u32,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate,
    };
    let environment = BTreeMap::new();
    let redis = connect(options, &environment)
        .await
        .expect("the cache opens without a server");
    Cache {
        backend: Backend::Redis(Box::new(redis)),
        cache_type: LiteLlmCacheType::Redis,
        namespace: None,
        redis_flush_size: None,
        ttl: None,
        mode: CacheMode::DefaultOn,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
    }
}
