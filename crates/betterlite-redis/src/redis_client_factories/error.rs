mod tests;

use std::num::ParseIntError;

use crate::redis_connection_settings::error::RedisSettingsError;

#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    #[error(transparent)]
    Settings(#[from] RedisSettingsError),
    #[error("Both 'sentinel_nodes' and 'service_name' are required for Redis Sentinel.")]
    SentinelTargetIncomplete,
    #[error("a url-targeted redis client requires 'url' to be set")]
    UrlTargetRequiresUrl,
    #[error("REDIS_CONNECTION_POOL_TIMEOUT is not a whole number of seconds: {source}")]
    InvalidPoolTimeout {
        #[source]
        source: ParseIntError,
    },
}
