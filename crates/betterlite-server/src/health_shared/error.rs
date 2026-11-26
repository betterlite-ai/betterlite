use betterlite_env::EnvError;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;

use crate::health_cycle::error::HealthCycleError;

// One cache call's failure: the operation the manager logs and the Redis cause it never raises.
#[derive(Debug, thiserror::Error)]
pub enum SharedHealthCacheError {
    #[error("the health check cache {operation} failed")]
    Read {
        operation: &'static str,
        #[source]
        source: RedisCacheError,
    },
    #[error("the health check cache {operation} failed")]
    Write {
        operation: &'static str,
        #[source]
        source: RedisCacheValuesError,
    },
    #[error("the health check cache {operation} failed")]
    Delete {
        operation: &'static str,
        #[source]
        source: RedisAdminError,
    },
}

// What perform_shared_health_check can raise: the probe's health cycle, kept as the source so the loop can tell what failed.
#[derive(Debug, thiserror::Error)]
pub enum SharedHealthError {
    #[error("the health check failed")]
    Cycle(#[source] HealthCycleError),
}

// A stored value that is not a health check result: json.loads' failure and the row shape around it.
#[derive(Debug, thiserror::Error)]
pub enum CachedHealthError {
    #[error("the cached health check results are not valid JSON")]
    Private(#[from] PrivateJsonError),
    #[error("the cached health check results are not health check result rows")]
    Shape(#[from] serde_json::Error),
}

// The two TTLs read from DEFAULT_SHARED_HEALTH_CHECK_TTL and DEFAULT_SHARED_HEALTH_CHECK_LOCK_TTL.
#[derive(Debug, thiserror::Error)]
pub enum SharedHealthSettingsError {
    #[error(transparent)]
    Environment(#[from] EnvError),
}
