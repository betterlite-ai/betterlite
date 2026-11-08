use super::error::RedisSemanticError;

pub const DEFAULT_REDIS_INDEX_NAME: &str = "litellm_semantic_cache_index";
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-ada-002";

pub struct RedisSemanticOptions {
    pub host: Option<String>,
    pub port: Option<String>,
    pub password: Option<String>,
    pub redis_url: Option<String>,
    pub similarity_threshold: Option<f64>,
    pub embedding_model: Option<String>,
    pub index_name: Option<String>,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: f64,
}

pub struct RedisEnv {
    pub redis_host: Option<String>,
    pub redis_port: Option<String>,
    pub redis_password: Option<String>,
}

pub(crate) fn resolve_redis_url(
    configured_url: Option<String>,
    configured_host: Option<String>,
    configured_port: Option<String>,
    configured_password: Option<String>,
    env: RedisEnv,
) -> Result<String, RedisSemanticError> {
    if let Some(url) = configured_url {
        return Ok(url);
    }
    let host = present(configured_host, env.redis_host).ok_or(
        RedisSemanticError::MissingConfiguration {
            variable: "REDIS_HOST",
        },
    )?;
    let port = present(configured_port, env.redis_port).ok_or(
        RedisSemanticError::MissingConfiguration {
            variable: "REDIS_PORT",
        },
    )?;
    let password = present(configured_password, env.redis_password).ok_or(
        RedisSemanticError::MissingConfiguration {
            variable: "REDIS_PASSWORD",
        },
    )?;
    Ok(format!("redis://:{password}@{host}:{port}"))
}

fn present(explicit: Option<String>, environment: Option<String>) -> Option<String> {
    explicit
        .filter(|value| !value.is_empty())
        .or_else(|| environment.filter(|value| !value.is_empty()))
}
