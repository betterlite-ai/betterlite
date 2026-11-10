pub mod error;
mod redis_state;
#[cfg(any(test, feature = "testing"))]
#[path = "routing_state_store/tests/scenario.rs"]
pub mod test_scenario;
#[cfg(any(test, feature = "testing"))]
#[path = "routing_state_store/tests/fake_redis.rs"]
pub mod test_support;
mod tests;

use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::routing_state_store as config;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::AsyncRedisCommands as _;
use redis::cmd;
use secrecy::SecretString;

use self::error::{Observed, StateStoreError};
use self::redis_state::RedisState;
use crate::redis_failure_policy;
use crate::routing_value_store::{CompareOutcome, RouteIncrement, RouteKey, RouteValue, StoreRead};

pub type BreakerClock = Arc<dyn Fn() -> Duration + Send + Sync>;

#[derive(Debug, Clone)]
pub struct RedisSettings {
    pub url: Option<SecretString>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub password: Option<SecretString>,
    pub namespace: Option<String>,
    pub socket_timeout: Option<f64>,
    pub pool_timeout_seconds: u32,
}

impl TryFrom<&config::Settings> for RedisSettings {
    type Error = StateStoreError;

    fn try_from(section: &config::Settings) -> Result<Self, StateStoreError> {
        if section.redis_url.is_none() && section.redis_host.is_none() {
            return Err(StateStoreError::NotConfigured);
        }
        Ok(Self {
            url: section.redis_url.clone(),
            host: section.redis_host.clone(),
            port: section.redis_port,
            password: section.redis_password.clone(),
            namespace: section.namespace.clone(),
            socket_timeout: Some(section.socket_timeout),
            pool_timeout_seconds: section.pool_timeout_seconds,
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

    pub async fn get(&self, key: &RouteKey) -> Result<Observed<StoreRead>, StateStoreError> {
        let namespaced = redis_state::namespaced(&self.redis, key);
        redis_state::run_observed(
            &self.redis,
            "async_get_cache",
            Some(StoreRead::Missing),
            move || async move {
                let client = self.redis.transport.async_client().await?;
                let mut command = cmd("GET");
                command.arg(&namespaced);
                let reply: Option<Vec<u8>> =
                    client
                        .query(&command)
                        .await
                        .map_err(|source| StateStoreError::Redis {
                            operation: "async_get_cache",
                            source,
                        })?;
                redis_state::codec::decode(reply)
            },
        )
        .await
    }

    pub async fn batch_get(
        &self,
        keys: &[RouteKey],
    ) -> Result<Observed<Vec<StoreRead>>, StateStoreError> {
        let namespaced: Vec<String> = keys
            .iter()
            .map(|key| redis_state::namespaced(&self.redis, key))
            .collect();
        let fallback = vec![StoreRead::Missing; keys.len()];
        redis_state::run_observed(
            &self.redis,
            "async_batch_get_cache",
            Some(fallback),
            move || async move {
                let replies = self.redis.transport.async_mget(&namespaced).await?;
                let mut values = Vec::with_capacity(replies.len());
                for reply in replies {
                    let raw = redis_state::codec::reply_bytes("async_batch_get_cache", reply)?;
                    values.push(redis_state::codec::decode(raw)?);
                }
                Ok(values)
            },
        )
        .await
    }

    pub async fn increment(
        &self,
        key: &RouteKey,
        by: &RouteIncrement,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<RouteValue>, StateStoreError> {
        let ttl = redis_state::typed_ttl(&self.redis, ttl_seconds)?;
        let namespaced = redis_state::namespaced(&self.redis, key);
        let key_name = key.as_ref().to_owned();
        redis_state::run_observed(&self.redis, "async_increment", None, move || async move {
            redis_state::typed::increment_until_written(&self.redis, &namespaced, by, ttl, key_name)
                .await
        })
        .await
    }

    pub async fn compare_and_set(
        &self,
        key: &RouteKey,
        expected: &StoreRead,
        value: &RouteValue,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<CompareOutcome>, StateStoreError> {
        let ttl = redis_state::typed_ttl(&self.redis, ttl_seconds)?;
        let namespaced = redis_state::namespaced(&self.redis, key);
        redis_state::run_observed(&self.redis, "compare_and_set", None, move || async move {
            redis_state::cas::compare_and_set(&self.redis, &namespaced, expected, value, ttl).await
        })
        .await
    }

    pub async fn increment_float(
        &self,
        key: &RouteKey,
        value: f64,
        ttl_seconds: Option<f64>,
        refresh_ttl: bool,
    ) -> Result<Observed<f64>, StateStoreError> {
        let ttl = redis_state::resolve_float_ttl(&self.redis, ttl_seconds);
        let namespaced = redis_state::namespaced(&self.redis, key);
        redis_state::run_observed(&self.redis, "async_increment", None, move || async move {
            self.redis
                .counters
                .incr_by_float_with_ttl(&namespaced, value, Some(ttl), refresh_ttl)
                .await
                .map_err(|source| StateStoreError::Counters {
                    operation: "async_increment",
                    source,
                })
        })
        .await
    }

    pub async fn increment_float_pipeline(
        &self,
        entries: &[(RouteKey, f64, Option<f64>)],
    ) -> Result<Observed<Option<Vec<f64>>>, StateStoreError> {
        if entries.is_empty() {
            return Ok(Observed {
                value: None,
                warning: None,
            });
        }
        let mut operations = Vec::with_capacity(entries.len());
        for (key, value, ttl_seconds) in entries {
            let ttl = match ttl_seconds {
                None => None,
                Some(seconds) => Some(
                    redis_state::truncate_seconds(*seconds)
                        .ok_or(StateStoreError::InvalidTtl { seconds: *seconds })?,
                ),
            };
            operations.push(IncrementOperation {
                key: redis_state::namespaced(&self.redis, key),
                increment_value: *value,
                ttl,
            });
        }
        redis_state::run_observed(
            &self.redis,
            "async_increment_pipeline",
            None,
            move || async move {
                self.redis
                    .counters
                    .pipeline_increment(&operations)
                    .await
                    .map(Some)
                    .map_err(|source| StateStoreError::Counters {
                        operation: "async_increment_pipeline",
                        source,
                    })
            },
        )
        .await
    }

    pub async fn get_ttl(&self, key: &RouteKey) -> Result<Observed<Option<i64>>, StateStoreError> {
        let namespaced = redis_state::namespaced(&self.redis, key);
        redis_state::run_observed(
            &self.redis,
            "async_get_ttl",
            Some(None),
            move || async move {
                let ttl = redis_state::typed::read_ttl(&self.redis, &namespaced).await?;
                Ok((ttl > -1).then_some(ttl))
            },
        )
        .await
    }

    pub async fn refresh_ttl(
        &self,
        key: &RouteKey,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<bool>, StateStoreError> {
        let ttl = redis_state::resolve_float_ttl(&self.redis, ttl_seconds);
        let namespaced = redis_state::namespaced(&self.redis, key);
        redis_state::run_observed(&self.redis, "async_refresh_ttl", None, move || async move {
            let client = self.redis.transport.async_client().await?;
            client
                .expire(&namespaced, ttl)
                .await
                .map_err(|source| StateStoreError::Redis {
                    operation: "async_refresh_ttl",
                    source,
                })
        })
        .await
    }
}
