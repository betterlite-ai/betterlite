use secrecy::{ExposeSecret as _, SecretString};

use super::error::RedisSemanticError;

pub const DEFAULT_REDIS_INDEX_NAME: &str = "litellm_semantic_cache_index";
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-ada-002";

pub struct RedisSemanticOptions {
    pub host: Option<String>,
    pub port: Option<String>,
    pub password: Option<SecretString>,
    pub redis_url: Option<SecretString>,
    pub similarity_threshold: Option<f64>,
    pub embedding_model: Option<String>,
    pub index_name: Option<String>,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: f64,
}

pub struct RedisEnv {
    pub redis_host: Option<String>,
    pub redis_port: Option<String>,
    pub redis_password: Option<SecretString>,
}

pub(crate) fn resolve_redis_url(
    configured_url: Option<SecretString>,
    configured_host: Option<String>,
    configured_port: Option<String>,
    configured_password: Option<SecretString>,
    env: RedisEnv,
) -> Result<SecretString, RedisSemanticError> {
    if let Some(url) = configured_url {
        return Ok(url);
    }
    let host = present(configured_host, env.redis_host, String::is_empty).ok_or(
        RedisSemanticError::MissingConfiguration {
            variable: "REDIS_HOST",
        },
    )?;
    let port = present(configured_port, env.redis_port, String::is_empty).ok_or(
        RedisSemanticError::MissingConfiguration {
            variable: "REDIS_PORT",
        },
    )?;
    let password = present(configured_password, env.redis_password, |value| {
        value.expose_secret().is_empty()
    })
    .ok_or(RedisSemanticError::MissingConfiguration {
        variable: "REDIS_PASSWORD",
    })?;
    Ok(SecretString::from(format!(
        "redis://:{}@{host}:{port}",
        password.expose_secret()
    )))
}

fn present<T>(explicit: Option<T>, environment: Option<T>, is_empty: fn(&T) -> bool) -> Option<T> {
    explicit
        .filter(|value| !is_empty(value))
        .or_else(|| environment.filter(|value| !is_empty(value)))
}
