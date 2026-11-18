use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::FloatOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_secret_managers::secret_lookup::text_to_bool;
use serde::de::Error as _;

use super::error::CoordRedisError;
use crate::proxy_request_contracts::coordination_redis::CoordinationRedisParams;
use crate::redis_cache_wiring::build::{RedisCacheRequest, RedisCacheWiring};

pub const REDIS_CLUSTER_NODES: &str = "REDIS_CLUSTER_NODES";
const REDIS_SENTINEL_NODES: &str = "REDIS_SENTINEL_NODES";
const REDIS_SOCKET_TIMEOUT: &str = "REDIS_SOCKET_TIMEOUT";

// litellm only branches on cluster mode for a non-empty node list; an empty or absent REDIS_CLUSTER_NODES keeps the plain client.
pub fn usage_family(
    params: &CoordinationRedisParams,
    env: &BTreeMap<String, String>,
) -> Result<RedisCacheFamily, CoordRedisError> {
    match params.startup_nodes.as_ref() {
        Some(nodes) if nodes.is_empty() => Ok(RedisCacheFamily::Standalone),
        Some(_) => Ok(RedisCacheFamily::Cluster),
        None => env_family(env),
    }
}

pub fn env_family(env: &BTreeMap<String, String>) -> Result<RedisCacheFamily, CoordRedisError> {
    let Some(text) = env
        .get(REDIS_CLUSTER_NODES)
        .filter(|text| text_to_bool(text.as_str()).is_none())
    else {
        return Ok(RedisCacheFamily::Standalone);
    };
    let parsed: JsonValue =
        serde_json::from_str(text).map_err(|source| CoordRedisError::ClusterNodes { source })?;
    Ok(if selects_cluster(&parsed) {
        RedisCacheFamily::Cluster
    } else {
        RedisCacheFamily::Standalone
    })
}

// get_secret_str turns the exact "true"/"false" texts into bools and then answers None, so such a value names no node list.
#[must_use]
pub fn env_has_redis_target(env: &BTreeMap<String, String>) -> bool {
    env.contains_key("REDIS_HOST")
        || env.contains_key("REDIS_URL")
        || env
            .get(REDIS_CLUSTER_NODES)
            .is_some_and(|text| text_to_bool(text.as_str()).is_none())
        || env
            .get(REDIS_SENTINEL_NODES)
            .is_some_and(|text| text_to_bool(text.as_str()).is_none())
}

pub async fn build_usage_redis(
    params: &CoordinationRedisParams,
    wiring: &RedisCacheWiring,
    env: &BTreeMap<String, String>,
) -> Result<RedisCache, CoordRedisError> {
    let family = usage_family(params, env)?;
    let request = usage_cache_request(params, family)?;
    wiring
        .build(request, env)
        .await
        .map_err(CoordRedisError::from)
}

pub async fn build_from_env(
    wiring: &RedisCacheWiring,
    env: &BTreeMap<String, String>,
) -> Result<Option<RedisCache>, CoordRedisError> {
    if !env_has_redis_target(env) {
        return Ok(None);
    }
    let family = env_family(env)?;
    let request = RedisCacheRequest {
        overrides: environment_overrides(env),
        family,
        namespace: None,
        redis_flush_size: None,
    };
    let cache = wiring.build(request, env).await?;
    Ok(Some(cache))
}

// litellm passes every REDIS_* connection value as an explicit kwarg, so a configured socket timeout reaches the client instead of the cache's five second default.
fn environment_overrides(env: &BTreeMap<String, String>) -> RedisOverrides {
    let mut overrides = RedisOverrides::default();
    if let Some(text) = env
        .get(REDIS_SOCKET_TIMEOUT)
        .filter(|text| !text.trim().is_empty())
    {
        overrides.socket_timeout = Some(FloatOrText::Text(text.clone()));
    }
    overrides
}

// litellm hands RedisCache every model_dump field, so the cache's own namespace and flush size come from the block and the client never sees them.
fn usage_cache_request(
    params: &CoordinationRedisParams,
    family: RedisCacheFamily,
) -> Result<RedisCacheRequest, CoordRedisError> {
    let mut carried = params.clone();
    carried.password = None;
    carried.url = None;
    carried.sentinel_password = None;
    let serialized =
        serde_json::to_value(&carried).map_err(|source| CoordRedisError::Overrides { source })?;
    let JsonValue::Object(fields) = serialized else {
        return Err(CoordRedisError::Overrides {
            source: serde_json::Error::custom("the coordination Redis params are not an object"),
        });
    };
    let settings = fields
        .into_iter()
        .filter(|(name, value)| {
            !value.is_null() && name != "namespace" && name != "redis_flush_size"
        })
        .collect();
    let mut overrides = serde_json::from_value::<RedisOverrides>(JsonValue::Object(settings))
        .map_err(|source| CoordRedisError::Overrides { source })?;
    overrides.password = params.password.clone().map(Some);
    overrides.url = params.url.clone().map(Some);
    overrides.sentinel_password = params.sentinel_password.clone();
    Ok(RedisCacheRequest {
        overrides,
        family,
        namespace: params
            .extra
            .get("namespace")
            .and_then(JsonValue::as_str)
            .map(str::to_owned),
        redis_flush_size: params
            .extra
            .get("redis_flush_size")
            .and_then(JsonValue::as_u64)
            .and_then(|size| usize::try_from(size).ok()),
    })
}

// An empty, zero, false or null JSON value in REDIS_CLUSTER_NODES names no nodes, so it selects the plain client.
fn selects_cluster(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_none_or(|value| value != 0.0),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}
