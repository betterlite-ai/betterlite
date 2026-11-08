mod buffer;
pub mod error;
mod events;
mod naming;
mod reads;
mod tests;
mod ttl;
mod writes;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use self::buffer::{buffered_entries, buffered_reaches_flush, remove_sent, serialize_entries};
use self::error::RedisCacheValuesError;
use self::events::{event, finish_outcome};
use self::naming::apply_namespace;
use self::reads::batch_values;
use self::writes::{
    execute_pipeline, one_ttl_commands, per_entry_commands, set_command, write_sadd,
};
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::transport::RedisTransport;

const DEFAULT_FLUSH_SIZE: usize = 100;

#[derive(Clone, Debug)]
pub struct RedisCacheValuesSettings {
    pub namespace: Option<String>,
    pub default_ttl: Option<i64>,
    pub redis_flush_size: Option<usize>,
}

pub struct RedisCacheValues {
    namespace: Option<String>,
    default_ttl: Option<i64>,
    flush_size: usize,
    transport: Arc<RedisTransport>,
    hooks: Arc<dyn ServiceHooks>,
    gate: Arc<dyn BreakerGate>,
    buffer: Mutex<Vec<(String, String)>>,
}

impl RedisCacheValues {
    #[must_use]
    pub fn new(
        settings: RedisCacheValuesSettings,
        transport: Arc<RedisTransport>,
        hooks: Arc<dyn ServiceHooks>,
        gate: Arc<dyn BreakerGate>,
    ) -> Self {
        Self {
            namespace: settings.namespace,
            default_ttl: settings.default_ttl,
            flush_size: settings.redis_flush_size.unwrap_or(DEFAULT_FLUSH_SIZE),
            transport,
            hooks,
            gate,
            buffer: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn buffered_count(&self) -> usize {
        match self.buffer.lock() {
            Ok(buffer) => buffer.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    pub fn set_cache<V>(&self, key: &str, value: &V, ttl: Option<f64>)
    where
        V: Serialize + ?Sized,
    {
        let started = Instant::now();
        let namespaced = apply_namespace(self, key);
        let Ok(command) = set_command(self.default_ttl, &namespaced, value, ttl, false) else {
            return;
        };
        if self.transport.sync_client.query::<()>(&command).is_err() {
            return;
        }
        let finished = event("set_cache", started, None);
        self.hooks.success(&finished);
    }

    pub async fn async_set_cache<V>(
        &self,
        key: Option<&str>,
        value: &V,
        ttl: Option<f64>,
        nx: bool,
    ) -> Result<Option<bool>, RedisCacheValuesError>
    where
        V: Serialize + ?Sized + Sync,
    {
        let started = Instant::now();
        let admission = self.gate.enter("async_set_cache")?;
        let Some(provided) = key else {
            self.gate.exit(admission);
            return Ok(None);
        };
        let namespaced = apply_namespace(self, provided);
        let client = match self.transport.async_client().await {
            Ok(client) => client,
            Err(source) => {
                let failed = event("async_set_cache", started, None);
                self.hooks.failure(&failed, &source);
                self.gate.record_failure(&source);
                return Err(RedisCacheValuesError::ClientReuse(source));
            }
        };
        let outcome = match set_command(self.default_ttl, &namespaced, value, ttl, nx) {
            Ok(command) => match client.query::<Option<String>>(&command).await {
                Ok(reply) => Ok(reply.map(|_stored| true)),
                Err(source) => Err(RedisCacheValuesError::Live(source)),
            },
            Err(error) => Err(error),
        };
        let finished = event("async_set_cache", started, Some(namespaced));
        Ok(finish_outcome(self, &finished, admission, outcome).flatten())
    }

    pub async fn async_set_cache_pipeline<V>(
        &self,
        cache_list: &[(String, V)],
        ttl: Option<f64>,
    ) -> Result<(), RedisCacheValuesError>
    where
        V: Serialize + Sync,
    {
        let started = Instant::now();
        let admission = self.gate.enter("async_set_cache_pipeline")?;
        if cache_list.is_empty() {
            self.gate.exit(admission);
            return Ok(());
        }
        let client = match self.transport.async_client().await {
            Ok(client) => client,
            Err(source) => {
                self.gate.record_failure(&source);
                return Err(RedisCacheValuesError::ClientReuse(source));
            }
        };
        let outcome = match one_ttl_commands(self, cache_list, ttl) {
            Ok(commands) => execute_pipeline(&client, commands).await,
            Err(error) => Err(error),
        };
        let finished = event("async_set_cache_pipeline", started, None);
        let _stored = finish_outcome(self, &finished, admission, outcome);
        Ok(())
    }

    // ast-grep-ignore: no-long-names
    pub async fn async_set_cache_pipeline_with_ttls<V>(
        &self,
        cache_list: &[(String, V, Option<f64>)],
    ) -> Result<(), RedisCacheValuesError>
    where
        V: Serialize + Sync,
    {
        let started = Instant::now();
        let admission = self.gate.enter("async_set_cache_pipeline_with_ttls")?;
        if cache_list.is_empty() {
            self.gate.exit(admission);
            return Ok(());
        }
        let outcome = match per_entry_commands(self, cache_list) {
            Ok(commands) => match self.transport.async_client().await {
                Ok(client) => execute_pipeline(&client, commands).await,
                Err(source) => Err(RedisCacheValuesError::ClientReuse(source)),
            },
            Err(error) => Err(error),
        };
        let finished = event("async_set_cache_pipeline_with_ttls", started, None);
        let _stored = finish_outcome(self, &finished, admission, outcome);
        Ok(())
    }

    pub async fn async_set_cache_sadd(
        &self,
        key: &str,
        value: &[String],
        ttl: Option<f64>,
    ) -> Result<(), RedisCacheValuesError> {
        let started = Instant::now();
        let admission = self.gate.enter("async_set_cache_sadd")?;
        let client = match self.transport.async_client().await {
            Ok(client) => client,
            Err(source) => {
                let failed = event("async_set_cache_sadd", started, None);
                self.hooks.failure(&failed, &source);
                self.gate.record_failure(&source);
                return Err(RedisCacheValuesError::ClientReuse(source));
            }
        };
        let namespaced = apply_namespace(self, key);
        let outcome = write_sadd(&client, &namespaced, value, ttl, self.default_ttl).await;
        let finished = event("async_set_cache_sadd", started, None);
        let _stored = finish_outcome(self, &finished, admission, outcome);
        Ok(())
    }

    pub async fn batch_cache_write<V>(
        &self,
        key: &str,
        value: &V,
    ) -> Result<(), RedisCacheValuesError>
    where
        V: Serialize + ?Sized + Sync,
    {
        let admission = self.gate.enter("batch_cache_write")?;
        let namespaced = apply_namespace(self, key);
        let text = match serde_json::to_string(value) {
            Ok(text) => text,
            Err(source) => {
                self.gate.record_failure(&source);
                return Err(RedisCacheValuesError::Serialize(source));
            }
        };
        if buffered_reaches_flush(self, namespaced, text)
            && let Err(error) = self.flush_cache_buffer().await
        {
            self.gate.record_failure(&error);
            return Err(error);
        }
        self.gate.exit(admission);
        Ok(())
    }

    pub async fn flush_cache_buffer(&self) -> Result<(), RedisCacheValuesError> {
        let entries = buffered_entries(self);
        let raw = serialize_entries(&entries)?;
        self.async_set_cache_pipeline(&raw, None).await?;
        remove_sent(self, raw.len());
        Ok(())
    }

    pub fn get_cache(&self, key: &str) -> Result<Option<CachedValue>, RedisCacheValuesError> {
        let started = Instant::now();
        let admission = self.gate.enter("get_cache")?;
        let namespaced = apply_namespace(self, key);
        let mut command = redis::cmd("GET");
        command.arg(&namespaced);
        let reply = match self
            .transport
            .sync_client
            .query::<Option<Vec<u8>>>(&command)
        {
            Ok(reply) => reply,
            Err(source) => {
                self.gate.swallowed(&admission, &source);
                self.gate.exit(admission);
                return Ok(None);
            }
        };
        let finished = event("get_cache", started, None);
        self.hooks.success(&finished);
        match decode_cached_value(reply.as_deref()) {
            Ok(value) => {
                self.gate.exit(admission);
                Ok(value)
            }
            Err(error) => {
                self.gate.swallowed(&admission, &error);
                self.gate.exit(admission);
                Ok(None)
            }
        }
    }

    pub fn batch_get_cache(
        &self,
        key_list: &[Option<&str>],
    ) -> Result<HashMap<String, Option<CachedValue>>, RedisCacheValuesError> {
        let originals: Vec<&str> = key_list.iter().filter_map(|key| *key).collect();
        let started = Instant::now();
        let admission = self.gate.enter("batch_get_cache")?;
        let namespaced: Vec<String> = originals
            .iter()
            .map(|key| apply_namespace(self, key))
            .collect();
        let finished = event("batch_get_cache", started, None);
        let replies = match self.transport.mget(&namespaced) {
            Ok(replies) => replies,
            Err(source) => {
                self.hooks.failure(&finished, &source);
                self.gate.swallowed(&admission, &source);
                return Ok(HashMap::new());
            }
        };
        self.gate.exit(admission);
        self.hooks.success(&finished);
        match batch_values(&originals, replies) {
            Ok(values) => Ok(values),
            Err(error) => {
                self.hooks.failure(&finished, &error);
                self.gate.swallowed(&admission, &error);
                Ok(HashMap::new())
            }
        }
    }
}
