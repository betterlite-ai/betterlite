mod tests;

use super::dispatch::credential_provider_auth;
use super::error::FactoryError;
use super::fields::ConnectionFields;
use super::url::{self, UrlClient};
use crate::redis_connection_settings::environment::EnvironmentLookup;
use crate::redis_connection_settings::model::settings::{RedisConnectionSettings, Target};

// litellm.constants.REDIS_CONNECTION_POOL_TIMEOUT: int(os.getenv("REDIS_CONNECTION_POOL_TIMEOUT", 5)).
const DEFAULT_POOL_TIMEOUT_SECONDS: u32 = 5;

// litellm._redis.get_redis_connection_pool: the pool carries the same connection fields as the client.
#[derive(Debug, Clone)]
pub enum PoolConnection {
    Direct(ConnectionFields),
    Url(UrlClient),
}

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub timeout_seconds: u32,
    pub max_connections: u64,
    pub connection: PoolConnection,
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
    if let Some(value) = settings.max_connections.filter(|value| *value < 1) {
        return Err(FactoryError::InvalidMaxConnections {
            value,
            source: None,
        });
    }
    let auth_settings = credential_provider_auth(settings);
    let max_connections = match auth_settings.max_connections {
        Some(value) => {
            u64::try_from(value).map_err(|source| FactoryError::InvalidMaxConnections {
                value,
                source: Some(source),
            })?
        }
        None => 50,
    };
    let connection = match target {
        Target::Cluster | Target::Sentinel => return Ok(None),
        Target::Url => PoolConnection::Url(url::build(&auth_settings)?),
        Target::Direct => PoolConnection::Direct(ConnectionFields::from(&auth_settings)),
    };
    Ok(Some(PoolConfig {
        timeout_seconds,
        max_connections,
        connection,
    }))
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
