use std::collections::BTreeMap;

use secrecy::ExposeSecret;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    CredentialProviderSettings, RedisConnectionSettings,
};

const CACHE_KEY_PREFIX: &str = "async-redis-client-";
const HASH_CHARACTERS: usize = 16;

// litellm.caching.redis_cache.RedisCache._get_async_client_cache_key: the hash stands in for sorted json of the kwargs. SecretString has no Serialize of its own, so the secret text is exposed here and only its digest leaves the function.
#[derive(Serialize)]
struct SettingsFingerprint<'settings> {
    host: &'settings Option<String>,
    port: Option<i64>,
    db: Option<i64>,
    username: &'settings Option<String>,
    password: Option<&'settings str>,
    url: Option<&'settings str>,
    ssl: Option<bool>,
    socket_timeout: Option<f64>,
    socket_connect_timeout: Option<f64>,
    socket_keepalive: Option<bool>,
    max_connections: Option<i64>,
    health_check_interval: Option<i64>,
    credential_provider: bool,
    credential_provider_settings: Option<CredentialFingerprint<'settings>>,
    startup_nodes: &'settings Option<Vec<ClusterNodeConfig>>,
    sentinel_nodes: &'settings Option<Vec<(String, i64)>>,
    sentinel_password: Option<&'settings str>,
    service_name: &'settings Option<String>,
    ssl_password: Option<&'settings str>,
    passthrough: &'settings BTreeMap<String, PassthroughValue>,
}

// The wire record has no Serialize of its own, and the key never leaves the process, so the tag text is ours.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CredentialFingerprint<'settings> {
    Explicit,
    Gcp {
        service_account: &'settings str,
        ssl_ca_certs: &'settings Option<String>,
    },
    Azure {
        client_id: &'settings Option<String>,
        tenant_id: &'settings Option<String>,
        client_secret: Option<&'settings str>,
    },
    Aws {
        user_name: &'settings str,
        cache_name: &'settings str,
        region: &'settings str,
        serverless: bool,
    },
}

impl<'settings> From<&'settings CredentialProviderSettings> for CredentialFingerprint<'settings> {
    fn from(settings: &'settings CredentialProviderSettings) -> Self {
        match settings {
            CredentialProviderSettings::Explicit => Self::Explicit,
            CredentialProviderSettings::Gcp {
                service_account,
                ssl_ca_certs,
            } => Self::Gcp {
                service_account,
                ssl_ca_certs,
            },
            CredentialProviderSettings::Azure {
                client_id,
                tenant_id,
                client_secret,
            } => Self::Azure {
                client_id,
                tenant_id,
                client_secret: client_secret.as_ref().map(ExposeSecret::expose_secret),
            },
            CredentialProviderSettings::Aws {
                user_name,
                cache_name,
                region,
                serverless,
            } => Self::Aws {
                user_name,
                cache_name,
                region,
                serverless: *serverless,
            },
        }
    }
}

#[must_use]
pub fn async_client_cache_key(settings: &RedisConnectionSettings) -> String {
    let fingerprint = SettingsFingerprint {
        host: &settings.host,
        port: settings.port,
        db: settings.db,
        username: &settings.username,
        password: settings.password.as_ref().map(ExposeSecret::expose_secret),
        url: settings.url.as_ref().map(ExposeSecret::expose_secret),
        ssl: settings.ssl,
        socket_timeout: settings.socket_timeout,
        socket_connect_timeout: settings.socket_connect_timeout,
        socket_keepalive: settings.socket_keepalive,
        max_connections: settings.max_connections,
        health_check_interval: settings.health_check_interval,
        credential_provider: settings.credential_provider,
        credential_provider_settings: settings
            .credential_provider_settings
            .as_ref()
            .map(CredentialFingerprint::from),
        startup_nodes: &settings.startup_nodes,
        sentinel_nodes: &settings.sentinel_nodes,
        sentinel_password: settings
            .sentinel_password
            .as_ref()
            .map(ExposeSecret::expose_secret),
        service_name: &settings.service_name,
        ssl_password: settings
            .ssl_password
            .as_ref()
            .map(ExposeSecret::expose_secret),
        passthrough: &settings.passthrough,
    };
    // Every field serializes without failing: the map keys are strings and a loaded config has no NaN.
    let serialized = serde_json::to_string(&fingerprint).unwrap_or_default();
    let digest = Sha256::digest(serialized.as_bytes());
    let hash = hex::encode(digest);
    let short = hash.get(..HASH_CHARACTERS).unwrap_or(&hash);
    format!("{CACHE_KEY_PREFIX}{short}")
}
