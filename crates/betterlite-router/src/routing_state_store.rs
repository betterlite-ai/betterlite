pub mod error;
mod redis_state;
#[cfg(any(test, feature = "testing"))]
#[path = "routing_state_store/tests/scenario.rs"]
pub mod test_scenario;
#[cfg(any(test, feature = "testing"))]
#[path = "routing_state_store/tests/fake_redis.rs"]
pub mod test_support;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::routing_state_store as config;
use betterlite_redis::cache_primitives::protocol::AsyncRedisCommands as _;
use secrecy::SecretString;

use self::error::{Observed, StateStoreError};
use self::redis_state::RedisState;
use crate::redis_failure_policy;
use crate::routing_value_store::{CompareOutcome, RouteIncrement, RouteKey, RouteValue, StoreRead};

pub type BreakerClock = Arc<dyn Fn() -> Duration + Send + Sync>;
pub type StoreResult<T> = Result<Observed<T>, StateStoreError>;

#[derive(Debug, Clone)]
pub struct RedisSettings {
    pub url: Option<SecretString>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub password: Option<SecretString>,
    pub namespace: Option<String>,
    pub connection: config::connection::Connection,
    pub socket_timeout: f64,
    pub pool_timeout_seconds: u32,
    pub environment: BTreeMap<String, SecretString>,
}

impl TryFrom<&config::Settings> for RedisSettings {
    type Error = StateStoreError;

    fn try_from(section: &config::Settings) -> Result<Self, StateStoreError> {
        let named_host = section.redis_host.is_some() && section.redis_port.is_some();
        if section.redis_url.is_none() && !named_host {
            return Err(StateStoreError::NotConfigured);
        }
        Ok(Self {
            url: section.redis_url.clone(),
            host: section.redis_host.clone(),
            port: section.redis_port,
            password: section.redis_password.clone(),
            namespace: section.namespace.clone(),
            connection: section.connection.clone(),
            socket_timeout: section.socket_timeout,
            pool_timeout_seconds: section.pool_timeout_seconds,
            environment: section.redis_environment.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub redis: RedisSettings,
    pub breaker: redis_failure_policy::Settings,
}

pub struct RoutingStateStore {
    redis: RedisState,
}

impl RoutingStateStore {
    pub fn new(settings: &Settings, clock: BreakerClock) -> Result<Self, StateStoreError> {
        Ok(Self {
            redis: redis_state::open(&settings.redis, settings.breaker, clock)?,
        })
    }

    #[rustfmt::skip]
    pub async fn get(&self, key: &RouteKey) -> StoreResult<StoreRead> {
        let state = &self.redis;
        let name = redis_state::namespaced(state, key);
        let read = move || async move { redis_state::typed::read_one(state, &name).await };
        redis_state::run_observed(state, "async_get_cache", Some(StoreRead::Missing), read).await
    }

    #[rustfmt::skip]
    pub async fn batch_get(&self, keys: &[RouteKey]) -> StoreResult<Vec<StoreRead>> {
        let state = &self.redis;
        let names: Vec<String> = keys.iter().map(|key| redis_state::namespaced(state, key)).collect();
        let fallback = vec![StoreRead::Missing; keys.len()];
        let read = move || async move { redis_state::typed::read_batch(state, &names).await };
        redis_state::run_observed(state, "async_batch_get_cache", Some(fallback), read).await
    }

    #[rustfmt::skip]
    pub async fn increment(&self, key: &RouteKey, by: &RouteIncrement, ttl_seconds: Option<f64>) -> StoreResult<RouteValue> {
        let state = &self.redis;
        let ttl = redis_state::typed_ttl(state, ttl_seconds)?;
        let name = redis_state::namespaced(state, key);
        let write = move || async move { redis_state::typed::increment_until_written(state, &name, by, ttl, key).await };
        redis_state::run_observed(state, "async_increment", None, write).await
    }

    #[rustfmt::skip]
    pub async fn compare_and_set(&self, key: &RouteKey, expected: &StoreRead, value: &RouteValue, ttl_seconds: Option<f64>) -> StoreResult<CompareOutcome> {
        let state = &self.redis;
        let ttl = redis_state::typed_ttl(state, ttl_seconds)?;
        let name = redis_state::namespaced(state, key);
        let swap = redis_state::cas::compare_and_set(state, &name, expected, value, redis_state::codec::Expiry::Seconds(ttl));
        redis_state::run_observed(state, "compare_and_set", None, move || swap).await
    }

    #[rustfmt::skip]
    pub async fn increment_float(&self, key: &RouteKey, value: f64, ttl_seconds: Option<f64>, refresh_ttl: bool) -> StoreResult<f64> {
        let state = &self.redis;
        let ttl = redis_state::resolve_float_ttl(state, ttl_seconds);
        let name = redis_state::namespaced(state, key);
        let add = move || async move { redis_state::typed::increment_float(state, &name, value, ttl, refresh_ttl).await };
        redis_state::run_observed(state, "async_increment", None, add).await
    }

    #[rustfmt::skip]
    pub async fn increment_float_pipeline(&self, entries: &[(RouteKey, f64, Option<f64>)]) -> StoreResult<Option<Vec<f64>>> {
        if entries.is_empty() {
            return Ok(Observed { value: None, warning: None });
        }
        let state = &self.redis;
        let operations = redis_state::typed::pipeline_operations(state, entries)?;
        let write = move || async move { redis_state::typed::increment_pipeline(state, &operations).await };
        let observed = redis_state::run_observed(state, "async_increment_pipeline", None, write).await?;
        Ok(Observed { value: Some(observed.value), warning: observed.warning })
    }

    #[rustfmt::skip]
    pub async fn get_ttl(&self, key: &RouteKey) -> StoreResult<Option<i64>> {
        let state = &self.redis;
        let name = redis_state::namespaced(state, key);
        let read = move || async move { redis_state::typed::read_ttl(state, &name).await.map(|ttl| (ttl > -1).then_some(ttl)) };
        redis_state::run_observed(state, "async_get_ttl", Some(None), read).await
    }

    #[rustfmt::skip]
    pub async fn refresh_ttl(&self, key: &RouteKey, ttl_seconds: Option<f64>) -> StoreResult<bool> {
        let state = &self.redis;
        let name = redis_state::namespaced(state, key);
        let ttl = redis_state::resolve_float_ttl(state, ttl_seconds);
        let expire = move || async move {
            let client = state.transport.async_client().await?;
            client.expire(&name, ttl).await.map_err(|source| StateStoreError::Redis { operation: "async_refresh_ttl", source })
        };
        redis_state::run_observed(state, "async_refresh_ttl", None, expire).await
    }
}
