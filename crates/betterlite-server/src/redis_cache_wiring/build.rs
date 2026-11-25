use std::sync::Arc;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_router::redis_failure_policy::gate::PolicyGate;
use betterlite_router::redis_failure_policy::{RedisFailurePolicy, Settings};
use betterlite_router::routing_state_store::BreakerClock;
use thiserror::Error;

// The settings the server resolves once and hands to every cache it builds.
pub struct RedisCacheWiring {
    pub factory: Arc<LiveClientFactory>,
    pub store: Arc<RedisClientStore>,
    pub hooks: Arc<dyn ServiceHooks>,
    pub policy: Settings,
    pub clock: BreakerClock,
    pub default_ttl: i64,
    pub default_major_version: u32,
}

// One cache's own settings, the part that differs between the response cache and the router's cache.
pub struct RedisCacheRequest {
    pub overrides: RedisOverrides,
    pub family: RedisCacheFamily,
    pub namespace: Option<String>,
    pub redis_flush_size: Option<usize>,
}

#[derive(Debug, Error)]
pub enum RedisCacheWiringError {
    #[error(transparent)]
    Connect(#[from] RedisCacheError),
}

impl RedisCacheWiring {
    pub async fn build<E>(
        &self,
        request: RedisCacheRequest,
        env: &E,
    ) -> Result<RedisCache, RedisCacheWiringError>
    where
        E: EnvironmentLookup + Sync,
    {
        let gate = Arc::new(PolicyGate::new(
            RedisFailurePolicy::new(self.policy, BreakerMetrics::new(None)),
            Arc::clone(&self.clock),
        ));
        connect(
            RedisCacheOptions {
                overrides: request.overrides,
                family: request.family,
                namespace: request.namespace,
                default_ttl: self.default_ttl,
                redis_flush_size: request.redis_flush_size,
                default_major_version: self.default_major_version,
                factory: Arc::clone(&self.factory),
                store: Arc::clone(&self.store),
                hooks: Arc::clone(&self.hooks),
                gate,
            },
            env,
        )
        .await
        .map_err(RedisCacheWiringError::from)
    }
}
