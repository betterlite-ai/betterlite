pub mod error;
mod index;
pub mod live;
mod lookup;
pub mod options;
pub mod ports;
mod schema;
mod store;

mod tests;

use std::sync::Arc;

use futures_util::future::join_all;
use secrecy::SecretString;
use tokio::sync::{Mutex, OnceCell};

use self::error::ValkeySemanticError;
use self::options::{DEFAULT_VALKEY_INDEX_NAME, ValkeyEnv, ValkeySemanticOptions};
use self::ports::{DocIdSource, UuidDocIds, ValkeyConnector, ValkeyIndex};
use crate::arguments::json::{Json, Object};
use crate::redis_semantic_cache::SemanticCacheKwargs;
use crate::redis_semantic_cache::options::DEFAULT_EMBEDDING_MODEL;
use crate::redis_semantic_cache::ports::SemanticEmbedder;

pub struct ValkeySemanticCache {
    pub embedding_model: String,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: f64,
    pub similarity_threshold: f64,
    pub index_name: String,
    pub key_prefix: String,
    pub url: Option<SecretString>,
    pub embedder: Arc<dyn SemanticEmbedder>,
    pub connector: Arc<dyn ValkeyConnector>,
    pub index: Option<Arc<dyn ValkeyIndex>>,
    pub doc_ids: Arc<dyn DocIdSource>,
    pub live: OnceCell<Arc<dyn ValkeyIndex>>,
    pub index_dim: Mutex<Option<usize>>,
}

impl ValkeySemanticCache {
    pub fn new(
        options: ValkeySemanticOptions,
        env: ValkeyEnv,
        embedder: Arc<dyn SemanticEmbedder>,
        connector: Arc<dyn ValkeyConnector>,
    ) -> Result<Self, ValkeySemanticError> {
        let Some(similarity_threshold) = options.similarity_threshold else {
            return Err(ValkeySemanticError::MissingSimilarityThreshold);
        };
        if !options.startup_nodes.is_empty() {
            return Err(ValkeySemanticError::ClusterModeUnsupported);
        }
        let index_name = options
            .index_name
            .clone()
            .unwrap_or_else(|| DEFAULT_VALKEY_INDEX_NAME.to_owned());
        tracing::debug!("Valkey semantic-cache initializing index - {index_name}");
        let key_prefix = format!("{index_name}:");
        let url = options
            .index
            .is_none()
            .then(|| options::resolve_url(&options, env))
            .transpose()?;
        Ok(Self {
            embedding_model: options
                .embedding_model
                .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned()),
            embedding_max_input_tokens: options.embedding_max_input_tokens,
            embedding_timeout: options.embedding_timeout,
            similarity_threshold,
            index_name,
            key_prefix,
            url,
            embedder,
            connector,
            index: options.index,
            doc_ids: Arc::new(UuidDocIds),
            live: OnceCell::new(),
            index_dim: Mutex::new(None),
        })
    }

    pub async fn async_set_cache(&self, key: &str, value: &Json, kwargs: &SemanticCacheKwargs) {
        if let Err(error) = store::store_value(self, key, value, kwargs).await {
            tracing::debug!("Error in async Valkey semantic-cache set_cache: {error}");
        }
    }

    pub async fn async_set_cache_pipeline(
        &self,
        cache_list: &[(String, Json)],
        kwargs: &SemanticCacheKwargs,
    ) {
        join_all(
            cache_list
                .iter()
                .map(|(key, value)| self.async_set_cache(key, value, kwargs)),
        )
        .await;
    }

    pub async fn async_get_cache(
        &self,
        key: &str,
        kwargs: &SemanticCacheKwargs,
        mut metadata: Option<&mut Object>,
    ) -> Option<Json> {
        match lookup::lookup(self, key, kwargs, &mut metadata).await {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!("Error in async Valkey semantic-cache get_cache: {error}");
                lookup::set_zero_similarity(&mut metadata);
                None
            }
        }
    }

    pub async fn index_info(&self) -> Result<Json, ValkeySemanticError> {
        index::backend(self)
            .await?
            .ft_info(self.index_name.as_str())
            .await
    }
}
