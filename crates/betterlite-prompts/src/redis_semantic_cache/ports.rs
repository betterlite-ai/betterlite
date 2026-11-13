use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use thiserror::Error;

use super::error::RedisSemanticError;
use crate::arguments::json::{Json, Object};

#[derive(Debug, Error)]
pub enum SemanticEmbedError {
    #[error("{message}")]
    Failed { message: String },
}

#[async_trait]
pub trait SemanticEmbedder: Send + Sync {
    async fn embed(
        &self,
        prompt: &str,
        metadata: Option<&Object>,
    ) -> Result<Vec<f32>, SemanticEmbedError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub prompt: Option<String>,
    pub response: Option<String>,
    pub litellm_cache_key: Option<String>,
    pub vector_distance: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchQuery {
    pub vector: Vec<f32>,
    pub distance_threshold: f64,
    pub cache_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntryFields {
    pub entry_id: String,
    pub prompt: String,
    pub response: String,
    pub prompt_vector: Vec<u8>,
    pub inserted_at: f64,
    pub updated_at: f64,
    pub litellm_cache_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSchema {
    pub name: String,
    pub prefix: String,
    pub dimension: usize,
}

#[async_trait]
pub trait SemanticRedisIndex: Send + Sync {
    async fn ft_info(&self, index: &str) -> Result<Option<Json>, RedisSemanticError>;
    async fn ft_create(&self, schema: &IndexSchema) -> Result<(), RedisSemanticError>;
    async fn ft_dropindex(&self, index: &str) -> Result<(), RedisSemanticError>;
    async fn ft_search(
        &self,
        index: &str,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<Hit>, RedisSemanticError>;
    async fn store_hash(
        &self,
        key: &str,
        fields: &CacheEntryFields,
        ttl: Option<i64>,
    ) -> Result<(), RedisSemanticError>;
}

#[async_trait]
pub trait SemanticIndexConnector: Send + Sync {
    async fn connect(
        &self,
        redis_url: &str,
    ) -> Result<Arc<dyn SemanticRedisIndex>, RedisSemanticError>;
}

pub trait SemanticClock: Send + Sync {
    fn now_seconds(&self) -> Result<f64, RedisSemanticError>;
}

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl SemanticClock for SystemClock {
    fn now_seconds(&self) -> Result<f64, RedisSemanticError> {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
            |_| Err(RedisSemanticError::ClockBeforeEpoch),
            |elapsed| Ok(elapsed.as_secs_f64()),
        )
    }
}
