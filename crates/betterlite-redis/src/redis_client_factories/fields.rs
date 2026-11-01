mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    CredentialProviderSettings, RedisConnectionSettings,
};

// litellm._redis._get_redis_kwargs: the kwargs shared by a direct client and a Sentinel data connection.
#[derive(Debug, Clone)]
pub struct ConnectionFields {
    pub host: Option<String>,
    pub port: Option<i64>,
    pub db: Option<i64>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub ssl: Option<bool>,
    pub socket_timeout: Option<f64>,
    pub socket_connect_timeout: Option<f64>,
    pub socket_keepalive: Option<bool>,
    pub max_connections: Option<i64>,
    pub health_check_interval: Option<i64>,
    pub credential_provider: bool,
    pub credential_provider_settings: Option<CredentialProviderSettings>,
    pub ssl_password: Option<SecretString>,
    pub passthrough: BTreeMap<String, PassthroughValue>,
}

impl From<&RedisConnectionSettings> for ConnectionFields {
    fn from(settings: &RedisConnectionSettings) -> Self {
        Self {
            host: settings.host.clone(),
            port: settings.port,
            db: settings.db,
            username: settings.username.clone(),
            password: settings.password.clone(),
            ssl: settings.ssl,
            socket_timeout: settings.socket_timeout,
            socket_connect_timeout: settings.socket_connect_timeout,
            socket_keepalive: settings.socket_keepalive,
            max_connections: settings.max_connections,
            health_check_interval: settings.health_check_interval,
            credential_provider: settings.credential_provider,
            credential_provider_settings: settings.credential_provider_settings.clone(),
            ssl_password: settings.ssl_password.clone(),
            passthrough: settings.passthrough.clone(),
        }
    }
}
