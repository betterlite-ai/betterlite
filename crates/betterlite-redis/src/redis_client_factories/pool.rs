mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use super::error::FactoryError;
use crate::redis_connection_settings::environment::EnvironmentLookup;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{RedisConnectionSettings, Target};

// litellm.constants.REDIS_CONNECTION_POOL_TIMEOUT: int(os.getenv("REDIS_CONNECTION_POOL_TIMEOUT", 5)).
const DEFAULT_POOL_TIMEOUT_SECONDS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolTarget {
    Direct,
    Url,
}

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub target: PoolTarget,
    pub timeout_seconds: u32,
    pub url: Option<SecretString>,
    pub max_connections: Option<i64>,
    pub ssl: Option<bool>,
    pub passthrough: BTreeMap<String, PassthroughValue>,
}

// litellm._redis.get_redis_connection_pool: no pool for Cluster; Sentinel also skips one (docs/deviations/B4323.yaml).
pub fn build_connection_pool<E>(
    settings: &RedisConnectionSettings,
    target: Target,
    env: &E,
) -> Result<Option<PoolConfig>, FactoryError>
where
    E: EnvironmentLookup,
{
    let timeout_seconds = pool_timeout_seconds(env)?;
    match target {
        Target::Cluster | Target::Sentinel => Ok(None),
        Target::Url => Ok(Some(PoolConfig {
            target: PoolTarget::Url,
            timeout_seconds,
            url: settings.url.clone(),
            max_connections: settings.max_connections,
            ssl: settings.ssl,
            passthrough: settings.passthrough.clone(),
        })),
        Target::Direct => Ok(Some(PoolConfig {
            target: PoolTarget::Direct,
            timeout_seconds,
            url: None,
            max_connections: settings.max_connections,
            ssl: settings.ssl,
            passthrough: settings.passthrough.clone(),
        })),
    }
}

fn pool_timeout_seconds<E>(env: &E) -> Result<u32, FactoryError>
where
    E: EnvironmentLookup,
{
    let Some(text) = env.lookup("REDIS_CONNECTION_POOL_TIMEOUT") else {
        return Ok(DEFAULT_POOL_TIMEOUT_SECONDS);
    };
    text.parse()
        .map_err(|source| FactoryError::InvalidPoolTimeout { source })
}
