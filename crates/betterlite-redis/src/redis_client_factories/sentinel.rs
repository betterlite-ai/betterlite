mod tests;

use secrecy::SecretString;

use super::error::FactoryError;
use super::fields::ConnectionFields;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;

// litellm._redis._sentinel_auth_kwargs: monitors get sentinel_password only, never the data node's credential provider or static password.
const SENTINEL_SOCKET_FALLBACK_SECS: f64 = 0.1;

#[derive(Debug, Clone)]
pub struct SentinelClient {
    pub sentinel_nodes: Vec<(String, i64)>,
    pub service_name: String,
    pub connection_kwargs: ConnectionFields,
    pub monitor_password: Option<SecretString>,
}

pub fn build(settings: &RedisConnectionSettings) -> Result<SentinelClient, FactoryError> {
    let sentinel_nodes = settings.sentinel_nodes.clone().unwrap_or_default();
    if sentinel_nodes.is_empty() {
        return Err(FactoryError::SentinelTargetIncomplete);
    }
    let Some(service_name) = settings.service_name.clone() else {
        return Err(FactoryError::SentinelTargetIncomplete);
    };

    let mut connection_kwargs = ConnectionFields::from(settings);
    if connection_kwargs.socket_timeout.is_none() {
        connection_kwargs.socket_timeout = Some(SENTINEL_SOCKET_FALLBACK_SECS);
    }

    Ok(SentinelClient {
        sentinel_nodes,
        service_name,
        connection_kwargs,
        monitor_password: settings.sentinel_password.clone(),
    })
}
