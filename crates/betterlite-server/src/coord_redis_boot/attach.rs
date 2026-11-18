use std::sync::{Arc, PoisonError};

use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::DualCache;
use betterlite_config::sections::settings::JsonValue;
use betterlite_redis::redis_cache::RedisCache;

// litellm's CLI SSO session cache keeps logins for ten minutes, whichever worker the browser lands on.
pub const CLI_SSO_SESSION_TTL_SECONDS: f64 = 600.0;

pub trait UsageCacheTarget: Sync {
    fn attach_redis(&self, redis: Arc<RedisCache>, default_ttl: Option<f64>);
}

impl<Memory: MemoryCacheOps> UsageCacheTarget for DualCache<Memory> {
    fn attach_redis(&self, redis: Arc<RedisCache>, default_ttl: Option<f64>) {
        self.attach_redis_cache(Some(redis), default_ttl);
    }
}

impl<Tier: AuthCacheTier> UsageCacheTarget for AuthCache<Tier> {
    fn attach_redis(&self, redis: Arc<RedisCache>, default_ttl: Option<f64>) {
        AuthCache::attach_redis_cache(self, Some(redis), default_ttl);
    }
}

// litellm assigns the coordination Redis to the config cache's field, replacing whatever it held before.
pub trait ConfigCacheTarget: Sync {
    fn replace_redis(&self, redis: Arc<RedisCache>);
}

impl<Memory: MemoryCacheOps> ConfigCacheTarget for DualCache<Memory> {
    fn replace_redis(&self, redis: Arc<RedisCache>) {
        let mut held = self
            .redis_cache
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *held = Some(redis);
    }
}

pub struct UsageCaches<'a> {
    pub spend_counter: &'a dyn UsageCacheTarget,
    pub cli_sso: &'a dyn UsageCacheTarget,
    pub config: &'a dyn ConfigCacheTarget,
    pub auth: &'a dyn UsageCacheTarget,
}

pub struct UsageAttach<'a> {
    pub caches: UsageCaches<'a>,
    pub default_redis_ttl: Option<f64>,
}

// litellm compares the setting with `is True`, so only the boolean true opts the auth cache in.
#[must_use]
pub fn enable_redis_auth_cache(litellm_settings: &JsonValue) -> bool {
    litellm_settings
        .get("enable_redis_auth_cache")
        .and_then(JsonValue::as_bool)
        == Some(true)
}

pub fn attach_usage_redis(
    redis: &Arc<RedisCache>,
    enable_auth_cache: bool,
    attach: &UsageAttach<'_>,
) {
    attach
        .caches
        .spend_counter
        .attach_redis(Arc::clone(redis), attach.default_redis_ttl);
    attach
        .caches
        .cli_sso
        .attach_redis(Arc::clone(redis), Some(CLI_SSO_SESSION_TTL_SECONDS));
    if enable_auth_cache {
        attach
            .caches
            .auth
            .attach_redis(Arc::clone(redis), attach.default_redis_ttl);
        tracing::info!(
            "enable_redis_auth_cache=True: attached Redis to user_api_key_cache — virtual-key lookups are now shared across all proxy workers."
        );
    } else {
        tracing::info!(
            "enable_redis_auth_cache is not set: user_api_key_cache remains in-memory only (per-worker). Set litellm_settings.enable_redis_auth_cache: true to share the auth cache across workers and reduce DB load."
        );
    }
    attach.caches.config.replace_redis(Arc::clone(redis));
}
