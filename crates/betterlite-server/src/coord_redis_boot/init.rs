use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_secret_managers::secret_lookup::text_to_bool;
use tokio::time::timeout;

use super::attach::{UsageAttach, attach_usage_redis};
use super::build::{build_from_env, build_usage_redis};
use super::env_refs::{ResolveContext, resolve_params};
use super::error::CoordRedisError;
use super::settings::{
    CoordinationGeneralSettings, CoordinationLitellmSettings, CoordinationRedisBlock,
    CoordinationRedisSettings,
};
use crate::proxy_request_contracts::coordination_redis::{BoolOrText, CoordinationRedisParams};
use crate::redis_cache_wiring::build::RedisCacheWiring;

pub(crate) const PING_TIMEOUT: Duration = Duration::from_secs(2);

#[async_trait]
pub trait UsageRedisProbe: Send + Sync {
    async fn ping(&self, redis: &RedisCache) -> Result<bool, RedisAdminError>;
}

pub struct AdminPing;

#[async_trait]
impl UsageRedisProbe for AdminPing {
    async fn ping(&self, redis: &RedisCache) -> Result<bool, RedisAdminError> {
        redis.admin.ping(None).await
    }
}

pub async fn config_coordination_params(
    settings: &CoordinationRedisSettings,
    context: &ResolveContext<'_>,
) -> Result<Option<CoordinationRedisParams>, CoordRedisError> {
    let Some(block) = settings.general_settings.coordination_redis.as_ref() else {
        return Ok(None);
    };
    let params = resolve_params(block, context).await?;
    if !params.has_connection_target() {
        return Err(CoordRedisError::MissingTarget);
    }
    Ok(Some(params))
}

pub async fn init_from_config(
    settings: &CoordinationRedisSettings,
    context: &ResolveContext<'_>,
    wiring: &RedisCacheWiring,
    attach: &UsageAttach<'_>,
) -> Result<Option<Arc<RedisCache>>, CoordRedisError> {
    let Some(params) = config_coordination_params(settings, context).await? else {
        return Ok(None);
    };
    let cache = Arc::new(build_usage_redis(&params, wiring, context.env).await?);
    let enable_auth = settings.litellm_settings.enable_redis_auth_cache == Some(true);
    attach_usage_redis(&cache, enable_auth, attach);
    tracing::info!(
        "coordination_redis: using a standalone Redis from general_settings for usage tracking, rate limiting, and cross-pod coordination."
    );
    Ok(Some(cache))
}

pub async fn init_from_db(
    persisted: Option<&CoordinationRedisBlock>,
    litellm_settings: &CoordinationLitellmSettings,
    context: &ResolveContext<'_>,
    wiring: &RedisCacheWiring,
    attach: &UsageAttach<'_>,
) -> Result<Option<Arc<RedisCache>>, CoordRedisError> {
    let Some(block) = persisted else {
        return Ok(None);
    };
    let params = resolve_params(block, context).await?;
    if !params.has_connection_target() {
        tracing::warn!(
            "coordination_redis saved in the database names no connection target; ignoring it."
        );
        return Ok(None);
    }
    let cache = Arc::new(build_usage_redis(&params, wiring, context.env).await?);
    attach_usage_redis(
        &cache,
        litellm_settings.enable_redis_auth_cache == Some(true),
        attach,
    );
    tracing::info!(
        "coordination_redis: using the standalone Redis saved in the database for usage tracking, rate limiting, and cross-pod coordination."
    );
    Ok(Some(cache))
}

// The inferred fallback never stops startup: a malformed value, an unreachable Redis and a false ping all stay in memory.
pub async fn init_env_fallback(
    litellm_settings: &CoordinationLitellmSettings,
    wiring: &RedisCacheWiring,
    env: &BTreeMap<String, String>,
    probe: &dyn UsageRedisProbe,
    attach: &UsageAttach<'_>,
) -> Option<Arc<RedisCache>> {
    let cache = match build_from_env(wiring, env).await {
        Ok(Some(cache)) => Arc::new(cache),
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(
                "coordination_redis: could not build a Redis client from REDIS_* environment variables ({error}); cross-pod coordination stays in-memory. Set general_settings.coordination_redis explicitly to require it."
            );
            return None;
        }
    };
    let reachable = match timeout(PING_TIMEOUT, probe.ping(&cache)).await {
        Ok(Ok(reachable)) => Ok(reachable),
        Ok(Err(error)) => Err(error.to_string()),
        Err(elapsed) => Err(elapsed.to_string()),
    };
    match reachable {
        Ok(true) => {}
        Ok(false) => return None,
        Err(error) => {
            tracing::warn!(
                "coordination_redis: REDIS_* environment variables named a Redis that is not reachable ({error}); cross-pod coordination stays in-memory. Set general_settings.coordination_redis explicitly to require it."
            );
            return None;
        }
    }
    let enable_auth = litellm_settings.enable_redis_auth_cache == Some(true);
    attach_usage_redis(&cache, enable_auth, attach);
    tracing::info!(
        "coordination_redis: using a standalone Redis built from REDIS_* environment variables for usage tracking, rate limiting, and cross-pod coordination."
    );
    Some(cache)
}

pub async fn buffer_redis(
    settings: &CoordinationGeneralSettings,
    usage_redis: Option<&Arc<RedisCache>>,
    wiring: &RedisCacheWiring,
    env: &BTreeMap<String, String>,
) -> Result<Option<Arc<RedisCache>>, CoordRedisError> {
    if let Some(redis) = usage_redis {
        return Ok(Some(Arc::clone(redis)));
    }
    if !buffer_wanted(settings.use_redis_transaction_buffer.as_ref()) {
        return Ok(None);
    }
    Ok(build_from_env(wiring, env).await?.map(Arc::new))
}

// litellm reads the flag as a bool or a string through str_to_bool, so "false" and "no" leave the buffer off.
fn buffer_wanted(flag: Option<&BoolOrText>) -> bool {
    match flag {
        Some(BoolOrText::Bool(value)) => *value,
        Some(BoolOrText::Text(text)) => text_to_bool(text).unwrap_or(false),
        None => false,
    }
}
