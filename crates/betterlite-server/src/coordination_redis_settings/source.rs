use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use serde::Serialize;

use crate::cache_settings_read::view::carries_value;
use crate::coord_redis_boot::build::env_has_redis_target;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationRedisSource {
    CoordinationRedis,
    CacheBackend,
    Environment,
}

// litellm tests the response cache against RedisCache and RedisClusterCache; the caller maps its own backend here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheBackendKind {
    Redis,
    Other,
}

// litellm's _coordination_redis_source: an explicit block, then the response cache's plain Redis client, then REDIS_*.
#[must_use]
pub fn coordination_redis_source(
    settings: Option<&JsonValue>,
    cache_backend: CacheBackendKind,
    environment: &BTreeMap<String, String>,
) -> Option<CoordinationRedisSource> {
    if settings.is_some_and(carries_value) {
        return Some(CoordinationRedisSource::CoordinationRedis);
    }
    if cache_backend == CacheBackendKind::Redis {
        return Some(CoordinationRedisSource::CacheBackend);
    }
    if env_has_redis_target(environment) {
        return Some(CoordinationRedisSource::Environment);
    }
    None
}
