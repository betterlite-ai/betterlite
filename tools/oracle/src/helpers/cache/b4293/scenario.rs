use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheSettings};
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::{
    active_request_redis_batch, request_redis_batch_scope,
};
use serde_json::json;

use super::input::{Call, Fields};
use super::server::{FakeRedis, Frame, connect_cache, set_expiries};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const EXPIRY: Duration = Duration::from_secs(1);
const LOCAL: CacheOptions = CacheOptions {
    ttl: None,
    local_only: true,
    refresh_ttl: false,
};

struct Scenario {
    cache: DualCache<LocalMemoryOps>,
    server: FakeRedis,
    redis: Arc<RedisCache>,
}

impl Scenario {
    async fn start(call: &Fields) -> Result<Self, Box<HelperError>> {
        let server = FakeRedis::start().await?;
        let redis = connect_cache(server.addr).await?;
        let reply = serde_json::to_string(&call.redis_get).or_value_error()?;
        server.script("GET", Frame::Bulk(reply));
        let clock = call.clock;
        let memory = LocalMemoryOps::new(1024, Arc::new(move || clock)).or_value_error()?;
        let settings = DualCacheSettings {
            default_in_memory_ttl: call.default_in_memory_ttl,
            ..DualCacheSettings::from_environment(&BTreeMap::<String, String>::new())
        };
        let cache = DualCache::new(Arc::new(memory), Some(Arc::clone(&redis)), settings);
        Ok(Self {
            cache,
            server,
            redis,
        })
    }

    async fn observe(&self, key: &str, read: Option<Json>) -> HelperResult {
        let ttl = self.cache.async_get_ttl(key).await.or_value_error()?;
        let second = self.cache.get_cache(key, false);
        Ok(json!({"read": read, "ttl": ttl, "second": second}))
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
    }
}

async fn read(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call).await?;
    for entry in &call.memory {
        scenario.cache.set_cache(&entry.key, &entry.value, LOCAL);
    }
    let read = scenario.cache.get_cache(&call.key, false);
    scenario.observe(&call.key, read).await
}

async fn scoped_read(scenario: &Scenario, key: &str) -> Option<Json> {
    let redis = Arc::clone(&scenario.redis);
    let (_batches, read) = request_redis_batch_scope(EXPIRY, async {
        let batch = active_request_redis_batch(&redis)?;
        let _misses = batch.mget(&[key.to_owned()]).await;
        scenario.cache.async_get_cache(key, false).await
    })
    .await;
    read
}

async fn batch_read(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call).await?;
    let read = scoped_read(&scenario, &call.key).await;
    scenario.observe(&call.key, read).await
}

async fn set_default_ttl(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call).await?;
    let options = CacheOptions::default();
    scenario.cache.set_cache(&call.key, &call.value, options);
    let memory_ttl = scenario
        .cache
        .async_get_ttl(&call.key)
        .await
        .or_value_error()?;
    Ok(json!({"memory_ttl": memory_ttl, "sets": set_expiries(&scenario.server)}))
}

async fn failing(call: &Fields, command: &str) -> Result<(Scenario, f64), Box<HelperError>> {
    let scenario = Scenario::start(call).await?;
    scenario.server.script(command, Frame::Failure("ERR boom"));
    let wire = serde_json::to_value(&call.value).or_value_error()?;
    let amount = wire.as_f64().ok_or("the increment is not a number");
    Ok((scenario, amount.or_value_error()?))
}

async fn increment_error(call: &Fields) -> HelperResult {
    let (scenario, amount) = failing(call, "INCRBY").await?;
    let outcome = scenario
        .cache
        .increment_cache(&call.key, amount, CacheOptions::default());
    let memory = scenario.cache.get_cache(&call.key, true);
    Ok(json!({"error": outcome.is_err(), "memory": memory}))
}

async fn increment_fallback(call: &Fields) -> HelperResult {
    let (scenario, amount) = failing(call, "INCRBYFLOAT").await?;
    let result = scenario
        .cache
        .async_increment_cache(&call.key, amount, CacheOptions::default());
    Ok(json!({"result": result.await}))
}

async fn pre_call(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call).await?;
    let cache = &scenario.cache;
    let outside = cache.set_cache_pre_call(&call.key, &call.value, None);
    let (_batches, inside) = request_redis_batch_scope(EXPIRY, async {
        cache.set_cache_pre_call(&call.key, &call.value, None)
    })
    .await;
    let queued = inside.or_value_error()?;
    let in_scope = queued.is_some();
    if let Some(result) = queued {
        let _settled = result.await;
    }
    let memory = cache.get_cache(&call.key, true);
    Ok(json!({
        "outside": outside.or_value_error()?.is_some(),
        "in_scope": in_scope,
        "memory": memory,
        "sets": set_expiries(&scenario.server),
    }))
}

async fn pipeline(call: &Fields) -> HelperResult {
    let scenario = Scenario::start(call).await?;
    let pairs: Vec<(String, Json)> = call
        .entries
        .iter()
        .map(|entry| (entry.key.clone(), entry.value.clone()))
        .collect();
    let options = CacheOptions::default();
    scenario
        .cache
        .async_set_cache_pipeline(&pairs, options)
        .await;
    let mut ttls = Vec::with_capacity(pairs.len());
    for (key, _value) in &pairs {
        ttls.push(scenario.cache.async_get_ttl(key).await.or_value_error()?);
    }
    Ok(json!({"ttls": ttls, "sets": set_expiries(&scenario.server)}))
}
