pub mod batch_read;
pub mod connect;
pub mod error;
mod increment;
mod reads;
mod tests;
pub mod ttl;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use redis::Value;

use self::error::{RedisCacheError, connection_failure};
use crate::cache_primitives::decode::CachedValue;
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::pipeline_increment::IncrementOperation;
use crate::cache_primitives::protocol::RedisScalar;
use crate::list_operations::RedisListOperations;
use crate::redis_cache_admin::RedisCacheAdmin;
use crate::redis_cache_values::RedisCacheValues;
use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::transport::RedisTransport;
use crate::redis_counters::RedisCounters;
use crate::redis_counters::scripts::RegisteredScript;

pub struct RedisCache {
    pub transport: Arc<RedisTransport>,
    pub values: RedisCacheValues,
    pub counters: RedisCounters,
    pub admin: Arc<RedisCacheAdmin>,
    pub lists: RedisListOperations,
    pub namespace: Option<String>,
    pub default_ttl: i64,
    pub redis_major_version: u32,
    pub hooks: Arc<dyn ServiceHooks>,
    pub gate: Arc<dyn BreakerGate>,
}

impl RedisCache {
    pub async fn async_increment(
        &self,
        key: &str,
        value: f64,
        ttl: Option<i64>,
        refresh_ttl: bool,
    ) -> Result<f64, RedisCacheError> {
        let started = Instant::now();
        let admission = self.gate.enter("async_increment")?;
        let used_ttl = ttl.unwrap_or(self.default_ttl);
        let lookup = self.transport.async_client().await;
        let outcome = match lookup {
            Ok(_client) => increment::increment(self, key, value, used_ttl, refresh_ttl).await,
            Err(source) if connection_failure(&source) => Err(RedisCacheError::from(source)),
            Err(source) => {
                return finish(
                    self.gate.as_ref(),
                    admission,
                    Err(RedisCacheError::from(source)),
                );
            }
        };
        let event = ServiceEvent {
            call_type: "async_increment",
            duration: started.elapsed(),
            key: None,
        };
        match &outcome {
            Ok(_incremented) => self.hooks.success(&event),
            Err(error) => {
                self.hooks.failure(&event, error);
                tracing::error!(
                    error = %error,
                    "LiteLLM Redis Caching: async async_increment() - Got exception from REDIS"
                );
            }
        }
        finish(self.gate.as_ref(), admission, outcome)
    }

    pub async fn async_increment_pipeline(
        &self,
        increment_list: &[IncrementOperation],
    ) -> Result<Option<Vec<f64>>, RedisCacheError> {
        let admission = self.gate.enter("async_increment_pipeline")?;
        if increment_list.is_empty() {
            self.gate.exit(admission);
            return Ok(None);
        }
        let started = Instant::now();
        let lookup = self.transport.async_client().await;
        let outcome = match lookup {
            // pipeline_increment namespaces its own keys, so it gets them as the caller wrote them
            Ok(_client) => self
                .counters
                .pipeline_increment(increment_list)
                .await
                .map_err(RedisCacheError::from),
            Err(source) if connection_failure(&source) => Err(RedisCacheError::from(source)),
            Err(source) => {
                return finish(
                    self.gate.as_ref(),
                    admission,
                    Err(RedisCacheError::from(source)),
                );
            }
        };
        let event = ServiceEvent {
            call_type: "async_increment_pipeline",
            duration: started.elapsed(),
            key: None,
        };
        match &outcome {
            Ok(_incremented) => self.hooks.success(&event),
            Err(error) => {
                self.hooks.failure(&event, error);
                tracing::error!(
                    error = %error,
                    "LiteLLM Redis Caching: async increment_pipeline() - Got exception from REDIS"
                );
            }
        }
        finish(self.gate.as_ref(), admission, outcome.map(Some))
    }

    pub async fn async_get_cache(&self, key: &str) -> Result<Option<CachedValue>, RedisCacheError> {
        let started = Instant::now();
        let admission = self.gate.enter("async_get_cache")?;
        let namespaced = self.namespaced(key);
        match reads::get_cached(self, &namespaced).await {
            reads::Read::Value(value) => {
                self.hooks.success(&ServiceEvent {
                    call_type: "async_get_cache",
                    duration: started.elapsed(),
                    key: Some(namespaced),
                });
                self.gate.exit(admission);
                Ok(value)
            }
            reads::Read::Client(error) => finish(self.gate.as_ref(), admission, Err(error)),
            reads::Read::Failure(error) => {
                self.hooks.failure(
                    &ServiceEvent {
                        call_type: "async_get_cache",
                        duration: started.elapsed(),
                        key: Some(namespaced),
                    },
                    &error,
                );
                tracing::debug!(error = %error, "async get() - Got exception from REDIS");
                self.gate.swallowed(&admission, &error);
                self.gate.exit(admission);
                Ok(None)
            }
        }
    }

    pub async fn async_batch_get_cache(
        &self,
        key_list: &[Option<&str>],
    ) -> Result<HashMap<String, Option<CachedValue>>, RedisCacheError> {
        let started = Instant::now();
        let admission = self.gate.enter("async_batch_get_cache")?;
        let keys = batch_read::batch_keys(key_list, self.namespace.as_deref());
        let namespaced: Vec<String> = keys.iter().map(|entry| entry.namespaced.clone()).collect();
        let replies = match self.transport.async_mget(&namespaced).await {
            Ok(replies) => replies,
            Err(source) => {
                return Ok(swallow_batch(
                    self,
                    admission,
                    started,
                    &RedisCacheError::from(source),
                ));
            }
        };
        self.hooks.success(&ServiceEvent {
            call_type: "async_batch_get_cache",
            duration: started.elapsed(),
            key: None,
        });
        match batch_read::read_replies(&keys, replies) {
            Ok(values) => {
                self.gate.exit(admission);
                Ok(values)
            }
            Err(error) => Ok(swallow_batch(self, admission, started, &error)),
        }
    }

    pub async fn async_get_ttl(&self, key: &str) -> Result<Option<i64>, RedisCacheError> {
        let admission = self.gate.enter("async_get_ttl")?;
        let namespaced = self.namespaced(key);
        Ok(ttl::read(self, &namespaced, admission).await)
    }

    pub async fn async_refresh_ttl(
        &self,
        key: &str,
        ttl: Option<i64>,
    ) -> Result<bool, RedisCacheError> {
        let admission = self.gate.enter("async_refresh_ttl")?;
        let namespaced = self.namespaced(key);
        let outcome = ttl::expire(self, &namespaced, ttl.unwrap_or(self.default_ttl)).await;
        finish(self.gate.as_ref(), admission, outcome)
    }

    pub async fn run_script(
        &self,
        script: &RegisteredScript,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<Value, RedisCacheError> {
        let admission = self.gate.enter("run_script")?;
        let outcome = script.run(keys, args).await.map_err(RedisCacheError::from);
        finish(self.gate.as_ref(), admission, outcome)
    }

    fn namespaced(&self, key: &str) -> String {
        check_and_fix_namespace(self.namespace.as_deref(), Some(key))
            .unwrap_or_else(|| key.to_owned())
    }
}

fn finish<T>(
    gate: &dyn BreakerGate,
    admission: BreakerAdmission,
    outcome: Result<T, RedisCacheError>,
) -> Result<T, RedisCacheError> {
    match outcome {
        Ok(value) => {
            gate.exit(admission);
            Ok(value)
        }
        Err(error) => {
            gate.record_failure(&error);
            Err(error)
        }
    }
}

fn swallow_batch(
    cache: &RedisCache,
    admission: BreakerAdmission,
    started: Instant,
    error: &RedisCacheError,
) -> HashMap<String, Option<CachedValue>> {
    cache.hooks.failure(
        &ServiceEvent {
            call_type: "async_batch_get_cache",
            duration: started.elapsed(),
            key: None,
        },
        error,
    );
    tracing::error!(error = %error, "Error occurred in async batch get cache");
    cache.gate.swallowed(&admission, error);
    cache.gate.exit(admission);
    HashMap::new()
}
