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

use self::buffer::{buffered_reaches_flush, remove_sent};
use self::error::RedisCacheValuesError;
use self::events::{
    client_or_swallowed, connection_failure, enter_unless_empty, event, finish_get, finish_outcome,
    open_client,
};
use self::naming::{apply_namespace, namespaced_keys};
use self::reads::{batch_values, read_value};
use self::writes::{send_shared_pipeline, send_ttls_pipeline, set_command, write_sadd};
use crate::cache_primitives::decode::CachedValue;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use serde_json::value::RawValue;

use crate::redis_client_reuse::transport::RedisTransport;

const DEFAULT_FLUSH_SIZE: usize = 100;

#[derive(Clone, Debug)]
pub struct RedisCacheValuesSettings {
    pub namespace: Option<String>,
    pub default_ttl: Option<i64>,
    pub redis_flush_size: Option<usize>,
}

pub struct RedisCacheValues {
    settings: RedisCacheValuesSettings,
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
            flush_size: settings.redis_flush_size.unwrap_or(DEFAULT_FLUSH_SIZE),
            settings,
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
        let Ok(command) = set_command(self.settings.default_ttl, &namespaced, value, ttl, false)
        else {
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
        let Some(client) = client_or_swallowed(
            self,
            "async_set_cache",
            started,
            admission,
            Some(namespaced.clone()),
        )
        .await?
        else {
            return Ok(None);
        };
        let outcome = match set_command(self.settings.default_ttl, &namespaced, value, ttl, nx) {
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
        let Some(admission) =
            enter_unless_empty(self, "async_set_cache_pipeline", cache_list.is_empty())?
        else {
            return Ok(());
        };
        let started = Instant::now();
        let client = match open_client(self).await {
            Ok(client) => client,
            Err(error) if connection_failure(&error) => {
                let failed = event("async_set_cache_pipeline", started, None);
                self.hooks.failure(&failed, &error);
                self.gate.swallowed(&admission, &error);
                self.gate.exit(admission);
                return Ok(());
            }
            Err(error) => {
                self.gate.record_failure(&error);
                return Err(error);
            }
        };
        let outcome = send_shared_pipeline(self, &client, cache_list, ttl).await;
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
        let admission = self.gate.enter("async_set_cache_pipeline_with_ttls")?;
        if cache_list.is_empty() {
            self.gate.exit(admission);
            return Ok(());
        }
        let started = Instant::now();
        let commands = self.prepare_ttl_commands(cache_list);
        send_ttls_pipeline(self, commands, admission, started).await
    }

    fn prepare_ttl_commands<V>(
        &self,
        cache_list: &[(String, V, Option<f64>)],
    ) -> Result<Vec<redis::Cmd>, RedisCacheValuesError>
    where
        V: Serialize,
    {
        cache_list
            .iter()
            .map(|(key, value, ttl)| {
                let namespaced = apply_namespace(self, key);
                set_command(self.settings.default_ttl, &namespaced, value, *ttl, false)
            })
            .collect()
    }

    pub async fn async_set_cache_sadd(
        &self,
        key: &str,
        value: &[String],
        ttl: Option<f64>,
    ) -> Result<(), RedisCacheValuesError> {
        let started = Instant::now();
        let admission = self.gate.enter("async_set_cache_sadd")?;
        let Some(client) =
            client_or_swallowed(self, "async_set_cache_sadd", started, admission, None).await?
        else {
            return Ok(());
        };
        let namespaced = apply_namespace(self, key);
        let outcome = write_sadd(&client, &namespaced, value, ttl, self.settings.default_ttl).await;
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
        let entries = match self.buffer.lock() {
            Ok(buffer) => buffer.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let raw = entries
            .iter()
            .map(|(key, text)| {
                RawValue::from_string(text.clone())
                    .map(|value| (key.clone(), value))
                    .map_err(RedisCacheValuesError::Serialize)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.async_set_cache_pipeline(&raw, None).await?;
        remove_sent(self, raw.len());
        Ok(())
    }

    pub fn get_cache(&self, key: &str) -> Result<Option<CachedValue>, RedisCacheValuesError> {
        let started = Instant::now();
        let admission = self.gate.enter("get_cache")?;
        let namespaced = apply_namespace(self, key);
        let reply = read_value(&self.transport, &namespaced);
        let finished = event("get_cache", started, None);
        Ok(finish_get(self, &finished, admission, reply))
    }

    pub fn batch_get_cache(
        &self,
        key_list: &[Option<&str>],
    ) -> Result<HashMap<String, Option<CachedValue>>, RedisCacheValuesError> {
        let originals: Vec<&str> = key_list.iter().filter_map(|key| *key).collect();
        let started = Instant::now();
        let admission = self.gate.enter("batch_get_cache")?;
        let namespaced = namespaced_keys(self, &originals);
        let replies = match self.transport.mget(&namespaced) {
            Ok(replies) => replies,
            Err(source) => {
                let failed = event("batch_get_cache", started, None);
                self.hooks.failure(&failed, &source);
                self.gate.swallowed(&admission, &source);
                return Ok(HashMap::new());
            }
        };
        self.gate.exit(admission);
        let loaded = event("batch_get_cache", started, None);
        self.hooks.success(&loaded);
        match batch_values(&originals, replies) {
            Ok(values) => Ok(values),
            Err(error) => {
                let failed = event("batch_get_cache", started, None);
                self.hooks.failure(&failed, &error);
                self.gate.swallowed(&admission, &error);
                Ok(HashMap::new())
            }
        }
    }
}
