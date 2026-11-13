use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::ValkeySemanticError;
use crate::arguments::json::Json;

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub response: String,
    pub distance: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexSchema {
    pub prefix: String,
    pub dimension: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnnQuery {
    pub text: String,
    pub return_fields: Vec<&'static str>,
    pub dialect: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocMapping {
    pub litellm_cache_key: String,
    pub prompt: String,
    pub response: String,
    pub embedding: Vec<u8>,
}

#[async_trait]
pub trait ValkeyIndex: Send + Sync {
    async fn ft_create(&self, index: &str, schema: &IndexSchema)
    -> Result<(), ValkeySemanticError>;

    async fn ft_info(&self, index: &str) -> Result<Json, ValkeySemanticError>;

    async fn ft_search(
        &self,
        index: &str,
        query: &KnnQuery,
        vector: &[u8],
    ) -> Result<Vec<Hit>, ValkeySemanticError>;

    async fn store_hash(
        &self,
        key: &str,
        fields: &DocMapping,
        ttl: Option<i64>,
    ) -> Result<(), ValkeySemanticError>;
}

#[async_trait]
pub trait ValkeyConnector: Send + Sync {
    async fn connect(
        &self,
        url: &SecretString,
    ) -> Result<Arc<dyn ValkeyIndex>, ValkeySemanticError>;
}

pub trait DocIdSource: Send + Sync {
    fn next_id(&self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UuidDocIds;

impl DocIdSource for UuidDocIds {
    fn next_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}
