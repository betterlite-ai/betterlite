use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use serde_json::Value;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::redis_semantic_cache::error::RedisSemanticError;
use betterlite_prompts::redis_semantic_cache::ports::{
    CacheEntryFields, Hit, IndexSchema, SemanticEmbedError, SemanticEmbedder,
    SemanticIndexConnector, SemanticRedisIndex, SemanticSearchQuery,
};

pub(crate) struct ConstantEmbedder {
    pub(crate) vector: Vec<f32>,
}

#[async_trait]
impl SemanticEmbedder for ConstantEmbedder {
    async fn embed(
        &self,
        _prompt: &str,
        _metadata: Option<&Object>,
    ) -> Result<Vec<f32>, SemanticEmbedError> {
        Ok(self.vector.clone())
    }
}

pub(crate) struct HelperConnector {
    pub(crate) index: Arc<dyn SemanticRedisIndex>,
}

#[async_trait]
impl SemanticIndexConnector for HelperConnector {
    async fn connect(
        &self,
        _redis_url: &str,
    ) -> Result<Arc<dyn SemanticRedisIndex>, RedisSemanticError> {
        Ok(Arc::clone(&self.index))
    }
}

#[derive(Default)]
pub(crate) enum Behavior {
    #[default]
    Recording,
    Scripted {
        mismatches: Vec<usize>,
    },
}

enum FallbackOp {
    Info(String),
    Drop(String),
}

#[derive(Default)]
pub(crate) struct HelperIndex {
    pub(crate) hits: Vec<Hit>,
    pub(crate) dimension: usize,
    behavior: Behavior,
    info_calls: Mutex<usize>,
    searches: Mutex<Vec<(String, SemanticSearchQuery)>>,
    stores: Mutex<Vec<(String, CacheEntryFields, Option<i64>)>>,
    operations: Mutex<Vec<FallbackOp>>,
}

impl HelperIndex {
    pub(crate) fn with_hits(hits: Vec<Hit>) -> Self {
        let mut index = Self::default();
        index.hits = hits;
        index
    }

    pub(crate) fn scripted(dimension: usize, mismatches: Vec<usize>) -> Self {
        let mut index = Self::default();
        index.dimension = dimension;
        index.behavior = Behavior::Scripted { mismatches };
        index
    }

    pub(crate) fn searches(&self) -> Vec<(String, SemanticSearchQuery)> {
        self.searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn stores(&self) -> Vec<(String, CacheEntryFields, Option<i64>)> {
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn attempts(&self) -> Vec<Value> {
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|operation| match operation {
                FallbackOp::Info(name) => serde_json::json!({ "name": name, "overwrite": false }),
                FallbackOp::Drop(name) => serde_json::json!({ "name": name, "overwrite": true }),
            })
            .collect()
    }
}

#[async_trait]
impl SemanticRedisIndex for HelperIndex {
    async fn ft_info(&self, index: &str) -> Result<Option<Json>, RedisSemanticError> {
        let Behavior::Scripted { mismatches } = &self.behavior else {
            return Ok(None);
        };
        let call = {
            let mut calls = self
                .info_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let call = *calls;
            *calls += 1;
            call
        };
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FallbackOp::Info(index.to_owned()));
        if mismatches.contains(&call) {
            // A reply that is not the expected schema forces the isolated-index fallback.
            return Ok(Some(Json::List(Vec::new())));
        }
        Ok(None)
    }

    async fn ft_create(&self, _schema: &IndexSchema) -> Result<(), RedisSemanticError> {
        Ok(())
    }

    async fn ft_dropindex(&self, index: &str) -> Result<(), RedisSemanticError> {
        self.operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FallbackOp::Drop(index.to_owned()));
        Ok(())
    }

    async fn ft_search(
        &self,
        index: &str,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<Hit>, RedisSemanticError> {
        self.searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((index.to_owned(), query.clone()));
        Ok(self.hits.clone())
    }

    async fn store_hash(
        &self,
        key: &str,
        fields: &CacheEntryFields,
        ttl: Option<i64>,
    ) -> Result<(), RedisSemanticError> {
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), fields.clone(), ttl));
        Ok(())
    }
}
