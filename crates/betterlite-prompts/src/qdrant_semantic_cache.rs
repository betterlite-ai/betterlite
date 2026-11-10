pub mod collection;
pub mod error;
pub mod live;
pub mod lookup;
pub mod options;
pub mod ports;
pub mod store;
mod tests;
mod wire;

use std::sync::Arc;

use futures_util::future::join_all;
use secrecy::SecretString;
use serde::de::DeserializeOwned;

use self::error::QdrantSemanticError;
use self::options::{
    DEFAULT_EMBEDDING_MODEL, DEFAULT_EMBEDDING_TIMEOUT, DEFAULT_SCALAR_QUANTILE,
    DEFAULT_VECTOR_SIZE, QdrantEnv, QdrantSemanticOptions, resolve_base, resolve_key,
};
use self::ports::{EmbeddingMode, QdrantEmbedder, QdrantTransport, UuidSource};
use crate::arguments::json::{Json, Object};
use crate::message_content::Message;
use crate::semantic_cache_records::extract_semantic_prompt;

pub struct QdrantSemanticCache {
    pub collection_name: String,
    pub similarity_threshold: f64,
    pub embedding_model: String,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: f64,
    pub vector_size: u32,
    pub scalar_quantile: f64,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    // Kept as configured so an existing collection is never held to the quantization validation
    pub quantization_config: Option<String>,
    pub embedder: Arc<dyn QdrantEmbedder>,
    pub transport: Arc<dyn QdrantTransport>,
    pub uuid: Arc<dyn UuidSource>,
}

pub struct QdrantCacheKwargs {
    pub messages: Option<Vec<Message>>,
    pub metadata: Option<Object>,
}

impl QdrantSemanticCache {
    pub fn new(
        options: QdrantSemanticOptions,
        env: &QdrantEnv,
        embedder: Arc<dyn QdrantEmbedder>,
        transport: Arc<dyn QdrantTransport>,
        uuid: Arc<dyn UuidSource>,
    ) -> Result<Self, QdrantSemanticError> {
        let Some(collection_name) = options.collection_name else {
            return Err(QdrantSemanticError::MissingCollectionName);
        };
        let Some(similarity_threshold) = options.similarity_threshold else {
            return Err(QdrantSemanticError::MissingSimilarityThreshold);
        };
        Ok(Self {
            collection_name,
            similarity_threshold,
            embedding_model: options
                .embedding_model
                .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned()),
            embedding_max_input_tokens: options.embedding_max_input_tokens,
            embedding_timeout: options
                .embedding_timeout
                .unwrap_or(DEFAULT_EMBEDDING_TIMEOUT),
            vector_size: options
                .vector_size
                .or(env.vector_size)
                .unwrap_or(DEFAULT_VECTOR_SIZE),
            scalar_quantile: options
                .scalar_quantile
                .or(env.scalar_quantile)
                .unwrap_or(DEFAULT_SCALAR_QUANTILE),
            api_base: resolve_base(options.qdrant_api_base, env)?,
            api_key: resolve_key(options.qdrant_api_key, env)?,
            quantization_config: options.quantization_config,
            embedder,
            transport,
            uuid,
        })
    }

    pub async fn initialize(&self) -> Result<(), QdrantSemanticError> {
        if !collection::collection_exists(self).await? {
            collection::create_collection(self).await?;
        }
        collection::read_collection_details(self).await?;
        collection::ensure_payload_index(self).await;
        Ok(())
    }

    pub async fn set_cache(
        &self,
        key: &str,
        value: &Json,
        kwargs: &QdrantCacheKwargs,
    ) -> Result<(), QdrantSemanticError> {
        let Some(prompt) = extract_semantic_prompt(kwargs.messages.as_deref(), None)? else {
            tracing::debug!("No prompt provided for semantic caching");
            return Ok(());
        };
        let vector = self
            .embedder
            .embed(&prompt, kwargs.metadata.as_ref(), EmbeddingMode::Sync)
            .await?;
        store::write_point(self, key, prompt, vector, value).await
    }

    pub async fn async_set_cache(
        &self,
        key: &str,
        value: &Json,
        kwargs: &QdrantCacheKwargs,
    ) -> Result<(), QdrantSemanticError> {
        let Some(prompt) = extract_semantic_prompt(kwargs.messages.as_deref(), None)? else {
            tracing::debug!("No prompt provided for semantic caching");
            return Ok(());
        };
        let vector = self
            .embedder
            .embed(&prompt, kwargs.metadata.as_ref(), EmbeddingMode::Async)
            .await?;
        store::write_point(self, key, prompt, vector, value).await
    }

    pub async fn async_set_cache_pipeline(
        &self,
        cache_list: &[(String, Json)],
        kwargs: &QdrantCacheKwargs,
    ) -> Result<(), QdrantSemanticError> {
        let writes = join_all(
            cache_list
                .iter()
                .map(|(key, value)| self.async_set_cache(key, value, kwargs)),
        )
        .await;
        writes.into_iter().collect()
    }

    pub async fn get_cache(
        &self,
        key: &str,
        kwargs: &QdrantCacheKwargs,
        metadata: Option<&mut Object>,
    ) -> Result<Option<Json>, QdrantSemanticError> {
        let Some(prompt) = extract_semantic_prompt(kwargs.messages.as_deref(), None)? else {
            tracing::debug!("No prompt provided for semantic cache lookup");
            return Ok(None);
        };
        let vector = self
            .embedder
            .embed(&prompt, kwargs.metadata.as_ref(), EmbeddingMode::Sync)
            .await?;
        lookup::search(self, key, vector, metadata).await
    }

    pub async fn async_get_cache(
        &self,
        key: &str,
        kwargs: &QdrantCacheKwargs,
        metadata: Option<&mut Object>,
    ) -> Result<Option<Json>, QdrantSemanticError> {
        let Some(prompt) = extract_semantic_prompt(kwargs.messages.as_deref(), None)? else {
            tracing::debug!("No prompt provided for semantic cache lookup");
            return Ok(None);
        };
        let vector = self
            .embedder
            .embed(&prompt, kwargs.metadata.as_ref(), EmbeddingMode::Async)
            .await?;
        lookup::search(self, key, vector, metadata).await
    }
}

pub(crate) fn decode<T>(body: &[u8], endpoint: &'static str) -> Result<T, QdrantSemanticError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(body)
        .map_err(|source| QdrantSemanticError::Response { endpoint, source })
}
