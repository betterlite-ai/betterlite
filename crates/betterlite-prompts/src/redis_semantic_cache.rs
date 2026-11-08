pub mod error;
pub mod index;
mod keys;
pub mod live;
mod lookup;
pub mod options;
pub mod ports;
mod query;
mod schema;
mod store;

mod tests;

use std::ops::Sub as _;
use std::sync::Arc;

use futures_util::future::join_all;
use secrecy::SecretString;
use tokio::sync::OnceCell;

use self::error::RedisSemanticError;
use self::index::SemanticIndex;
use self::options::{
    DEFAULT_EMBEDDING_MODEL, DEFAULT_REDIS_INDEX_NAME, RedisEnv, RedisSemanticOptions,
    resolve_redis_url,
};
use self::ports::{Hit, SemanticClock, SemanticEmbedder, SemanticIndexConnector, SystemClock};
use crate::arguments::json::{Json, Object};
use crate::message_content::Message;

pub const CACHE_KEY_FIELD_NAME: &str = "litellm_cache_key";
pub const VECTOR_FIELD_NAME: &str = "prompt_vector";
pub const SIMILARITY_FIELD: &str = "semantic-similarity";

pub struct SemanticCacheKwargs {
    pub messages: Option<Vec<Message>>,
    pub input: Option<Json>,
    pub metadata: Option<Object>,
    pub ttl: Option<i64>,
}

pub struct RedisSemanticCache {
    pub embedding_model: String,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: f64,
    pub similarity_threshold: f64,
    pub distance_threshold: f64,
    pub index_name: String,
    pub clock: Arc<dyn SemanticClock>,
    pub redis_url: SecretString,
    pub embedder: Arc<dyn SemanticEmbedder>,
    pub connector: Arc<dyn SemanticIndexConnector>,
    pub index: OnceCell<SemanticIndex>,
}

impl RedisSemanticCache {
    pub fn new(
        options: RedisSemanticOptions,
        env: RedisEnv,
        embedder: Arc<dyn SemanticEmbedder>,
        connector: Arc<dyn SemanticIndexConnector>,
    ) -> Result<Self, RedisSemanticError> {
        let Some(similarity_threshold) = options.similarity_threshold else {
            return Err(RedisSemanticError::MissingSimilarityThreshold);
        };
        let index_name = options
            .index_name
            .unwrap_or_else(|| DEFAULT_REDIS_INDEX_NAME.to_owned());
        tracing::debug!("Redis semantic-cache initializing index - {index_name}");
        let embedding_model = options
            .embedding_model
            .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned());
        let redis_url = resolve_redis_url(
            options.redis_url,
            options.host,
            options.port,
            options.password,
            env,
        )?;
        Ok(Self {
            embedding_model,
            embedding_max_input_tokens: options.embedding_max_input_tokens,
            embedding_timeout: options.embedding_timeout,
            similarity_threshold,
            distance_threshold: 1.0_f64.sub(similarity_threshold),
            index_name,
            clock: Arc::new(SystemClock),
            redis_url: SecretString::from(redis_url),
            embedder,
            connector,
            index: OnceCell::new(),
        })
    }

    // LiteLLM's _get_ttl returns the ttl as given; RedisVL's "ttl or self._ttl" reads 0 as no expiry.
    #[must_use]
    pub const fn get_ttl(ttl: Option<i64>) -> Option<i64> {
        match ttl {
            Some(0) => None,
            other => other,
        }
    }

    #[must_use]
    pub fn cache_hit_matches_key(hit: &Hit, key: &str) -> bool {
        // An entry without the tag field predates the isolation fix and cannot be scoped.
        let Some(cached) = hit.litellm_cache_key.as_deref() else {
            return false;
        };
        cached == key
    }

    pub async fn async_set_cache(&self, key: &str, value: &Json, kwargs: &SemanticCacheKwargs) {
        if let Err(error) = store::store_value(self, key, value, kwargs).await {
            tracing::debug!("Error in async_set_cache: {error}");
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
    ) -> Result<Option<Json>, RedisSemanticError> {
        match lookup::lookup(self, key, kwargs, &mut metadata).await {
            Ok(value) => Ok(value),
            Err(error) => {
                tracing::debug!("Error in async_get_cache: {error}");
                lookup::set_zero_similarity(&mut metadata);
                Ok(None)
            }
        }
    }

    pub async fn index_info(&self) -> Result<Json, RedisSemanticError> {
        let index = self
            .index
            .get_or_try_init(|| index::build_index(self))
            .await?;
        let Some(reply) = index.backend.ft_info(&index.name).await? else {
            return Err(RedisSemanticError::MissingIndex {
                index: index.name.clone(),
            });
        };
        schema::info_object(&reply)
    }
}
