use std::sync::Arc;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ValkeySemanticError;
use super::ports::ValkeyIndex;

pub const DEFAULT_VALKEY_INDEX_NAME: &str = "litellm_semantic_cache_index";

// Python's quote(password, safe='') keeps only these four punctuation marks unescaped
const PYTHON_QUOTE_UNSAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

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
    let resolved_host = first_set(
        settings.host.as_deref(),
        valkey_host.as_deref(),
        redis_host.as_deref(),
    );
    let resolved_port = first_set(
        settings.port.as_deref(),
        valkey_port.as_deref(),
        redis_port.as_deref(),
    );
    let (Some(host), Some(port)) = (resolved_host, resolved_port) else {
        return Err(ValkeySemanticError::MissingConfiguration);
    };
    let password = settings
        .password
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty())
        .or_else(|| {
            valkey_password
                .as_ref()
                .filter(|secret| !secret.expose_secret().is_empty())
        })
        .or_else(|| {
            redis_password
                .as_ref()
                .filter(|secret| !secret.expose_secret().is_empty())
        });
    Ok(build_valkey_url(host, port, password, settings.ssl))
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

fn build_valkey_url(
    host: &str,
    port: &str,
    password: Option<&SecretString>,
    ssl: bool,
) -> SecretString {
    let scheme = if ssl { "rediss" } else { "redis" };
    let Some(secret) = password.filter(|secret| !secret.expose_secret().is_empty()) else {
        return SecretString::from(format!("{scheme}://{host}:{port}"));
    };
    let encoded = percent_encode(secret.expose_secret().as_bytes(), PYTHON_QUOTE_UNSAFE);
    SecretString::from(format!("{scheme}://:{encoded}@{host}:{port}"))
}
