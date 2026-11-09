pub mod error;
mod redis_state;
mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::routing_state_store as config;
use secrecy::SecretString;

use self::error::{Observed, StateStoreError};
use self::redis_state::RedisState;
use crate::redis_failure_policy;
use crate::routing_value_store::{
    CompareOutcome, RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

pub type BreakerClock = Arc<dyn Fn() -> Duration + Send + Sync>;

const DEFAULT_REDIS_TTL: i64 = 60;

#[derive(Debug, Clone)]
pub struct RedisSettings {
    pub url: Option<SecretString>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub password: Option<SecretString>,
    pub namespace: Option<String>,
    pub socket_timeout: Option<f64>,
    pub pool_timeout_seconds: u32,
    pub default_ttl: i64,
}

impl TryFrom<&config::Settings> for RedisSettings {
    type Error = StateStoreError;

    fn try_from(section: &config::Settings) -> Result<Self, StateStoreError> {
        if section.redis_url.is_none() && section.redis_host.is_none() {
            return Err(StateStoreError::RedisRequired {
                operation: "configure",
            });
        }
        Ok(Self {
            url: section.redis_url.clone(),
            host: section.redis_host.clone(),
            port: section.redis_port,
            password: section.redis_password.clone(),
            namespace: section.namespace.clone(),
            socket_timeout: Some(section.socket_timeout),
            pool_timeout_seconds: section.pool_timeout_seconds,
            default_ttl: DEFAULT_REDIS_TTL,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub redis: Option<RedisSettings>,
    pub breaker: redis_failure_policy::Settings,
}

#[derive(Clone, Debug)]
pub struct FloatIncrement {
    pub key: RouteKey,
    pub value: f64,
    pub ttl_seconds: Option<f64>,
}

pub struct RoutingStateStore {
    local: RoutingValueStore,
    redis: Mutex<Option<Arc<RedisState>>>,
    breaker: redis_failure_policy::Settings,
    clock: BreakerClock,
}

impl RoutingStateStore {
    pub fn new(
        local: RoutingValueStore,
        settings: &Settings,
        clock: BreakerClock,
    ) -> Result<Self, StateStoreError> {
        let store = Self {
            local,
            redis: Mutex::new(None),
            breaker: settings.breaker,
            clock,
        };
        if let Some(redis) = settings.redis.as_ref() {
            store.attach_redis(redis)?;
        }
        Ok(store)
    }

    pub fn attach_redis(&self, settings: &RedisSettings) -> Result<(), StateStoreError> {
        if self.backend().is_some() {
            return Ok(());
        }
        let state = Arc::new(redis_state::open(
            settings,
            self.breaker,
            Arc::clone(&self.clock),
        )?);
        let mut backend = self.redis.lock().unwrap_or_else(PoisonError::into_inner);
        if backend.is_none() {
            *backend = Some(state);
        }
        drop(backend);
        Ok(())
    }

    fn backend(&self) -> Option<Arc<RedisState>> {
        self.redis
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub async fn get(&self, key: &RouteKey) -> Result<Observed<StoreRead>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(self.local.get(key)?));
        };
        redis_state::typed::get(&redis, key).await
    }

    pub async fn batch_get(
        &self,
        keys: &[RouteKey],
    ) -> Result<Observed<Vec<StoreRead>>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(self.local.batch_get(keys)?));
        };
        redis_state::typed::batch_get(&redis, keys).await
    }

    pub async fn set(
        &self,
        key: &RouteKey,
        value: &RouteValue,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<()>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(self.local.set(key, value, ttl_seconds)?));
        };
        redis_state::typed::set(&redis, key, value, ttl_seconds).await
    }

    pub async fn set_pipeline(
        &self,
        entries: &[(RouteKey, RouteValue)],
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<()>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(
                self.local.set_pipeline(entries, ttl_seconds)?,
            ));
        };
        redis_state::typed::set_pipeline(&redis, entries, ttl_seconds).await
    }

    pub async fn set_add(
        &self,
        key: &RouteKey,
        values: &[String],
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<Vec<String>>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(self.local.set_add(
                key,
                values,
                ttl_seconds,
            )?));
        };
        redis_state::typed::set_add(&redis, key, values, ttl_seconds).await
    }

    pub async fn delete(&self, key: &RouteKey) -> Result<Observed<()>, StateStoreError> {
        let Some(redis) = self.backend() else {
            self.local.delete(key);
            return Ok(local_observed(()));
        };
        redis_state::typed::delete(&redis, key).await
    }

    pub async fn increment(
        &self,
        key: &RouteKey,
        by: &RouteIncrement,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<RouteValue>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(self.local.increment(
                key,
                by,
                ttl_seconds,
            )?));
        };
        redis_state::typed::increment(&redis, key, by, ttl_seconds).await
    }

    pub async fn compare_and_set(
        &self,
        key: &RouteKey,
        expected: &StoreRead,
        value: &RouteValue,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<CompareOutcome>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Ok(local_observed(self.local.compare_and_set(
                key,
                expected,
                value,
                ttl_seconds,
            )?));
        };
        redis_state::typed::compare_and_set(&redis, key, expected, value, ttl_seconds).await
    }

    pub async fn increment_float(
        &self,
        key: &RouteKey,
        value: f64,
        ttl_seconds: Option<f64>,
        refresh_ttl: bool,
    ) -> Result<Observed<f64>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Err(StateStoreError::RedisRequired {
                operation: "async_increment",
            });
        };
        redis_state::float::increment(&redis, key, value, ttl_seconds, refresh_ttl).await
    }

    pub async fn increment_float_pipeline(
        &self,
        entries: &[FloatIncrement],
    ) -> Result<Observed<Option<Vec<f64>>>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Err(StateStoreError::RedisRequired {
                operation: "async_increment_pipeline",
            });
        };
        redis_state::float::increment_pipeline(&redis, entries).await
    }

    pub async fn get_ttl(&self, key: &RouteKey) -> Result<Observed<Option<i64>>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Err(StateStoreError::RedisRequired {
                operation: "async_get_ttl",
            });
        };
        redis_state::typed::get_ttl(&redis, key).await
    }

    pub async fn refresh_ttl(
        &self,
        key: &RouteKey,
        ttl_seconds: Option<f64>,
    ) -> Result<Observed<bool>, StateStoreError> {
        let Some(redis) = self.backend() else {
            return Err(StateStoreError::RedisRequired {
                operation: "async_refresh_ttl",
            });
        };
        redis_state::typed::refresh_ttl(&redis, key, ttl_seconds).await
    }
}

const fn local_observed<T>(value: T) -> Observed<T> {
    Observed {
        value,
        warning: None,
    }
}
