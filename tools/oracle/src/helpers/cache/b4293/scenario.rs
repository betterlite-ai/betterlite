use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{
    CacheOptions, DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE,
};
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::{
    active_request_redis_batch, request_redis_batch_scope,
};
use serde_json::{Value, json};
use thiserror::Error;

use super::input::{Call, Fields};
use super::server::{FakeRedis, Frame, connect_cache, delete_lengths, set_expiries};
use crate::helpers::{HelperError, HelperResult};

const EXPIRY: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error("cannot start the fake Redis server: {0}")]
    Server(#[source] std::io::Error),
    #[error("cannot connect the fake Redis cache: {0}")]
    Cache(String),
}

impl From<std::io::Error> for Failure {
    fn from(source: std::io::Error) -> Self {
        Self::Server(source)
    }
}

pub(crate) fn invalid<M>(message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

const fn write_options() -> CacheOptions {
    CacheOptions {
        ttl: None,
        local_only: false,
        refresh_ttl: false,
    }
}

const fn local_options() -> CacheOptions {
    CacheOptions {
        ttl: None,
        local_only: true,
        refresh_ttl: false,
    }
}

fn number(value: &Json) -> Result<f64, Box<HelperError>> {
    match value {
        Json::Number(count) => count
            .as_f64()
            .ok_or_else(|| invalid("the increment is out of range")),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(invalid("the increment is not a number"))
        }
    }
}

fn key(call: &Fields) -> Result<&str, Box<HelperError>> {
    call.key
        .as_deref()
        .ok_or_else(|| invalid("the call needs a key"))
}

struct Scenario {
    cache: DualCache<LocalMemoryOps>,
    server: FakeRedis,
    redis: Arc<RedisCache>,
}

impl Scenario {
    async fn start(clock: f64, memory_ttl: Option<f64>) -> Result<Self, Box<HelperError>> {
        let server = FakeRedis::start()
            .await
            .map_err(|failure| invalid(failure.to_string()))?;
        let redis = connect_cache(server.addr)
            .await
            .map_err(|failure| invalid(failure.to_string()))?;
        let memory = LocalMemoryOps::new(1024, Arc::new(move || clock))
            .map_err(|error| invalid(error.to_string()))?;
        let settings = DualCacheSettings {
            default_in_memory_ttl: memory_ttl,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        };
        let cache = DualCache::new(Arc::new(memory), Some(Arc::clone(&redis)), settings);
        Ok(Self {
            cache,
            server,
            redis,
        })
    }

    fn script_get(&self, value: &Value) -> Result<(), Box<HelperError>> {
        let text = serde_json::to_string(value).map_err(|error| invalid(error.to_string()))?;
        self.server.script("GET", Frame::Bulk(text));
        Ok(())
    }

    async fn ttl(&self, key: &str) -> Result<Option<f64>, Box<HelperError>> {
        self.cache
            .async_get_ttl(key)
            .await
            .map_err(|error| invalid(error.to_string()))
    }
}

pub(crate) async fn run(call: &Call) -> HelperResult {
    match call {
        Call::GetHitsMemoryFirst(fields) | Call::GetBackfillsMemoryFromRedis(fields) => {
            read(fields).await
        }
        Call::BatchMissingRead(fields) => batch_read(fields).await,
        Call::SetCacheAppliesDefaultTtl(fields) => set_default_ttl(fields).await,
        Call::IncrementPropagatesRedisError(fields) => increment_error(fields).await,
        Call::IncrementFallback(fields) => increment_fallback(fields).await,
        Call::SetPreCallReturnsBatchOrNone(fields) => pre_call(fields).await,
        Call::SetPipelineAppliesSingleTtl(fields) => pipeline(fields).await,
        Call::DeleteCacheKeysChunksRedis(fields) => delete_keys(fields).await,
    }
}

async fn read(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, call.default_in_memory_ttl).await?;
    scenario.script_get(&call.redis_get)?;
    for entry in &call.memory {
        scenario
            .cache
            .set_cache(&entry.key, &entry.value, local_options());
    }
    let key = key(call)?;
    let read = scenario.cache.get_cache(key, false);
    let ttl = scenario.ttl(key).await?;
    let second = scenario.cache.get_cache(key, false);
    Ok(json!({"read": read, "ttl": ttl, "second": second}))
}

async fn batch_read(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, call.default_in_memory_ttl).await?;
    scenario.script_get(&call.redis_get)?;
    let key = key(call)?;
    let redis = Arc::clone(&scenario.redis);
    let cache = &scenario.cache;
    let (_batches, read) = request_redis_batch_scope(EXPIRY, async {
        let batch = active_request_redis_batch(&redis)?;
        let _misses = batch.mget(&[key.to_owned()]).await;
        cache.async_get_cache(key, false).await
    })
    .await;
    let ttl = scenario.ttl(key).await?;
    let second = scenario.cache.get_cache(key, false);
    Ok(json!({"read": read, "ttl": ttl, "second": second}))
}

async fn set_default_ttl(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, call.default_in_memory_ttl).await?;
    let key = key(call)?;
    scenario.cache.set_cache(key, &call.value, write_options());
    let memory_ttl = scenario.ttl(key).await?;
    Ok(json!({"memory_ttl": memory_ttl, "sets": set_expiries(&scenario.server)}))
}

async fn increment_error(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, None).await?;
    scenario.server.script("INCRBY", Frame::Failure("ERR boom"));
    let key = key(call)?;
    let failed = scenario
        .cache
        .increment_cache(key, number(&call.value)?, write_options())
        .is_err();
    let memory = scenario.cache.get_cache(key, true);
    Ok(json!({"error": failed, "memory": memory}))
}

async fn increment_fallback(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, None).await?;
    scenario
        .server
        .script("INCRBYFLOAT", Frame::Failure("ERR boom"));
    let key = key(call)?;
    let result = scenario
        .cache
        .async_increment_cache(key, number(&call.value)?, write_options())
        .await;
    Ok(json!({"result": result}))
}

async fn pre_call(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, call.default_in_memory_ttl).await?;
    let key = key(call)?;
    let stored = call.value.clone();
    let outside = scenario
        .cache
        .set_cache_pre_call(key, &stored, None)
        .map_err(|error| invalid(error.to_string()))?;
    let cache = &scenario.cache;
    let (_batches, deferred) = request_redis_batch_scope(EXPIRY, async {
        cache.set_cache_pre_call(key, &stored, None)
    })
    .await;
    let queued = deferred.map_err(|error| invalid(error.to_string()))?;
    let in_scope = queued.is_some();
    if let Some(result) = queued {
        let _settled = result.await;
    }
    let memory = scenario.cache.get_cache(key, true);
    Ok(json!({
        "outside": outside.is_some(),
        "in_scope": in_scope,
        "memory": memory,
        "sets": set_expiries(&scenario.server),
    }))
}

async fn pipeline(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, call.default_in_memory_ttl).await?;
    let mut pairs: Vec<(String, Json)> = Vec::with_capacity(call.entries.len());
    for entry in &call.entries {
        pairs.push((entry.key.clone(), entry.value.clone()));
    }
    scenario
        .cache
        .async_set_cache_pipeline(&pairs, write_options())
        .await;
    let mut ttls = Vec::with_capacity(pairs.len());
    for (key, _value) in &pairs {
        ttls.push(scenario.ttl(key).await?);
    }
    Ok(json!({"ttls": ttls, "sets": set_expiries(&scenario.server)}))
}

async fn delete_keys(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call.clock, None).await?;
    let keys: Vec<String> = (0..call.key_count)
        .map(|index| format!("key-{index}"))
        .collect();
    let value = Json::Text("value".to_owned());
    for key in &keys {
        scenario.cache.set_cache(key, &value, local_options());
    }
    scenario
        .cache
        .async_delete_cache_keys(&keys)
        .await
        .map_err(|error| invalid(error.to_string()))?;
    let remaining = keys
        .iter()
        .filter(|key| scenario.cache.get_cache(key, true).is_some())
        .count();
    Ok(json!({"chunks": delete_lengths(&scenario.server), "remaining": remaining}))
}
