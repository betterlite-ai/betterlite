use serde::Serialize;

use super::RedisAdminError;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_factories::dispatch::build_client;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::settings::{ClientKind, RedisConnectionSettings};
use crate::redis_live_client::LiveClientFactory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionTestResult {
    pub status: ConnectionStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// litellm.caching.redis_cache.RedisCache.test_connection and RedisClusterCache.test_connection
#[must_use]
pub fn connection_test_result(
    family: RedisCacheFamily,
    ping: Result<bool, RedisAdminError>,
) -> ConnectionTestResult {
    match (family, ping) {
        (RedisCacheFamily::Standalone, Ok(true)) => ConnectionTestResult {
            status: ConnectionStatus::Success,
            message: "Redis connection test successful".to_owned(),
            error: None,
        },
        (RedisCacheFamily::Standalone, Ok(false)) => ConnectionTestResult {
            status: ConnectionStatus::Failed,
            message: "Redis ping returned False".to_owned(),
            error: None,
        },
        (RedisCacheFamily::Standalone, Err(error)) => ConnectionTestResult {
            status: ConnectionStatus::Failed,
            message: format!("Redis connection failed: {error}"),
            error: Some(error.to_string()),
        },
        (RedisCacheFamily::Cluster, Ok(true)) => ConnectionTestResult {
            status: ConnectionStatus::Success,
            message: "Redis Cluster connection test successful".to_owned(),
            error: None,
        },
        (RedisCacheFamily::Cluster, Ok(false)) => ConnectionTestResult {
            status: ConnectionStatus::Failed,
            message: "Redis Cluster ping returned False".to_owned(),
            error: None,
        },
        (RedisCacheFamily::Cluster, Err(error)) => ConnectionTestResult {
            status: ConnectionStatus::Failed,
            message: format!("Redis Cluster connection failed: {error}"),
            error: Some(error.to_string()),
        },
    }
}

// a fresh client and no pool, so the test proves the credentials rather than a cached connection
pub(super) async fn fresh_connection_ping(
    factory: &LiveClientFactory,
    settings: &RedisConnectionSettings,
) -> Result<bool, RedisAdminError> {
    let client = build_client(settings, ClientKind::Async).map_err(ClientReuseError::from)?;
    let opened = factory.open_async(&client, None).await?;
    let answer = opened.ping().await?;
    drop(opened);
    Ok(answer)
}
