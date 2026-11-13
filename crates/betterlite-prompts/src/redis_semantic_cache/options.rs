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
    let host = configured_host
        .filter(|value| !value.is_empty())
        .or(env.redis_host)
        .ok_or(RedisSemanticError::MissingConfiguration {
            variable: "REDIS_HOST",
        })?;
    let port = configured_port
        .filter(|value| !value.is_empty())
        .or(env.redis_port)
        .ok_or(RedisSemanticError::MissingConfiguration {
            variable: "REDIS_PORT",
        })?;
    let password = configured_password
        .filter(|value| !value.expose_secret().is_empty())
        .or(env.redis_password)
        .ok_or(RedisSemanticError::MissingConfiguration {
            variable: "REDIS_PASSWORD",
        })?;
    Ok(SecretString::from(format!(
        "redis://:{}@{host}:{port}",
        password.expose_secret()
    )))
}
