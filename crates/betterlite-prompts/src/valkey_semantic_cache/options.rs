use std::sync::Arc;

use betterlite_redis::valkey::build_valkey_url;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ValkeySemanticError;
use super::ports::ValkeyIndex;

pub const DEFAULT_VALKEY_INDEX_NAME: &str = "litellm_semantic_cache_index";

pub struct ValkeyEnv {
    pub valkey_host: Option<String>,
    pub valkey_port: Option<String>,
    pub valkey_password: Option<SecretString>,
    pub redis_host: Option<String>,
    pub redis_port: Option<String>,
    pub redis_password: Option<SecretString>,
}

pub struct ValkeySemanticOptions {
    pub host: Option<String>,
    pub port: Option<String>,
    pub password: Option<SecretString>,
    pub redis_url: Option<SecretString>,
    pub similarity_threshold: Option<f64>,
    pub embedding_model: Option<String>,
    pub index_name: Option<String>,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: f64,
    pub ssl: bool,
    pub startup_nodes: Vec<String>,
    pub index: Option<Arc<dyn ValkeyIndex>>,
}

pub(crate) fn resolve_url(
    settings: &ValkeySemanticOptions,
    env: ValkeyEnv,
) -> Result<SecretString, ValkeySemanticError> {
    let ValkeyEnv {
        valkey_host,
        valkey_port,
        valkey_password,
        redis_host,
        redis_port,
        redis_password,
    } = env;
    if let Some(url) = &settings.redis_url {
        return Ok(url.clone());
    }
    let Some(host) = first_set(
        settings.host.as_deref(),
        valkey_host.as_deref(),
        redis_host.as_deref(),
    ) else {
        return Err(ValkeySemanticError::MissingConfiguration);
    };
    let Some(port) = first_set(
        settings.port.as_deref(),
        valkey_port.as_deref(),
        redis_port.as_deref(),
    ) else {
        return Err(ValkeySemanticError::MissingConfiguration);
    };
    let port_number = parse_port(port)?;
    let password = first_secret(
        settings.password.as_ref(),
        valkey_password.as_ref(),
        redis_password.as_ref(),
    );
    Ok(build_valkey_url(host, port_number, password, settings.ssl))
}

fn parse_port(port: &str) -> Result<u16, ValkeySemanticError> {
    port.parse::<u16>()
        .map_err(|source| ValkeySemanticError::InvalidPort {
            port: port.to_owned(),
            source,
        })
}

fn first_set<'value>(
    explicit: Option<&'value str>,
    valkey: Option<&'value str>,
    redis: Option<&'value str>,
) -> Option<&'value str> {
    explicit
        .filter(|value| !value.is_empty())
        .or_else(|| valkey.filter(|value| !value.is_empty()))
        .or_else(|| redis.filter(|value| !value.is_empty()))
}

fn first_secret<'secret>(
    explicit: Option<&'secret SecretString>,
    valkey: Option<&'secret SecretString>,
    redis: Option<&'secret SecretString>,
) -> Option<&'secret SecretString> {
    explicit
        .filter(|secret| !secret.expose_secret().is_empty())
        .or_else(|| valkey.filter(|secret| !secret.expose_secret().is_empty()))
        .or_else(|| redis.filter(|secret| !secret.expose_secret().is_empty()))
}
