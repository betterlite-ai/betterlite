mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use super::error::FactoryError;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    CredentialProviderSettings, RedisConnectionSettings,
};

// litellm._redis: a "url" kwarg calls Redis.from_url with the URL allowlist kwargs.
#[derive(Debug, Clone)]
pub struct UrlClient {
    pub url: SecretString,
    pub username: Option<String>,
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

pub fn build(settings: &RedisConnectionSettings) -> Result<UrlClient, FactoryError> {
    let Some(url) = settings.url.clone() else {
        return Err(FactoryError::UrlTargetRequiresUrl);
    };
    Ok(UrlClient {
        url,
        username: settings.username.clone(),
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
    })
}
