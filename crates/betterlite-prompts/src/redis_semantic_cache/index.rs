use std::sync::Arc;

use secrecy::ExposeSecret as _;

use super::RedisSemanticCache;
use super::error::RedisSemanticError;
use super::ports::{IndexSchema, SemanticRedisIndex};
use super::schema::schema_matches;

pub struct SemanticIndex {
    pub backend: Arc<dyn SemanticRedisIndex>,
    pub name: String,
    pub dimension: usize,
}

pub(crate) async fn build_index(
    cache: &RedisSemanticCache,
) -> Result<SemanticIndex, RedisSemanticError> {
    match build_index_once(cache).await {
        Ok(index) => Ok(index),
        Err(error) => {
            tracing::error!("Redis semantic-cache index build failed: {error}");
            Err(error)
        }
    }
}

async fn build_index_once(cache: &RedisSemanticCache) -> Result<SemanticIndex, RedisSemanticError> {
    let probe = cache.embedder.embed("dimension test", None).await?;
    let backend = cache
        .connector
        .connect(cache.redis_url.expose_secret())
        .await?;
    let base = IndexSchema {
        name: cache.index_name.clone(),
        prefix: cache.index_name.clone(),
        dimension: probe.len(),
    };
    let name = build_index_chain(&*backend, &base).await?;
    Ok(SemanticIndex {
        backend,
        name,
        dimension: probe.len(),
    })
}

async fn build_index_chain(
    backend: &dyn SemanticRedisIndex,
    base: &IndexSchema,
) -> Result<String, RedisSemanticError> {
    match ensure_named_index(backend, base).await {
        Ok(()) => Ok(base.name.clone()),
        Err(RedisSemanticError::SchemaMismatch { .. }) => {
            let isolated = format!("{}_isolated", base.name);
            let schema = IndexSchema {
                name: isolated.clone(),
                prefix: isolated.clone(),
                dimension: base.dimension,
            };
            tracing::debug!(
                "Redis semantic-cache existing index schema is not isolated; using isolated index - {isolated}"
            );
            match ensure_named_index(backend, &schema).await {
                Ok(()) => Ok(isolated),
                Err(RedisSemanticError::SchemaMismatch { .. }) => {
                    tracing::debug!(
                        "Redis semantic-cache isolated index schema is stale; recreating isolated index - {isolated}"
                    );
                    backend.ft_dropindex(&isolated).await?;
                    backend.ft_create(&schema).await?;
                    Ok(isolated)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

async fn ensure_named_index(
    backend: &dyn SemanticRedisIndex,
    schema: &IndexSchema,
) -> Result<(), RedisSemanticError> {
    match backend.ft_info(&schema.name).await? {
        None => backend.ft_create(schema).await,
        Some(reply) => {
            if schema_matches(&reply, schema) {
                Ok(())
            } else {
                Err(RedisSemanticError::SchemaMismatch {
                    index: schema.name.clone(),
                })
            }
        }
    }
}
