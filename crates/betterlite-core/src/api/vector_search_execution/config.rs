use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::vector_stores::Headers;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::error::{DirectFailure, SearchError};
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};

pub type Params = BTreeMap<String, Json>;
pub type PreparedSearch = (String, Params);
pub type ResponseHeaders = BTreeMap<String, String>;

// A direct store and a query-embedding executor each raise their own error, and the handler passes it through without HTTP conversion
pub type StoreFailure = Box<dyn Error + Send + Sync>;

// LiteLLM's str | Sequence[str] query
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchQuery {
    Text(String),
    Sequence(Vec<String>),
}

// The provider's logging handle; the search transforms read none of it, and the field stays for the ones that record into it
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VectorStoreLogging {
    pub model_call_details: Params,
}

// BaseVectorStoreConfig.transform_search_vector_store_request's arguments
pub struct SearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub api_base: &'request str,
    pub logging: &'request mut VectorStoreLogging,
    pub litellm_params: &'request Params,
    pub extra_body: Option<&'request Params>,
}

// BaseQueryEmbeddingVectorStoreConfig transforms carry the executor beside the base arguments
pub struct QuerySearchRequest<'request> {
    pub search: SearchRequest<'request>,
    pub embedding_executor: Option<&'request dyn QueryEmbeddingExecutor>,
}

pub struct SignRequest<'request> {
    pub headers: &'request Headers,
    pub optional_params: &'request Params,
    pub request_data: &'request Params,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
}

pub struct VectorStoreResponse {
    pub status_code: u16,
    pub headers: ResponseHeaders,
    pub body: Vec<u8>,
}

// The BaseVectorStoreConfig members the search handler calls; a provider decides what each one does
#[async_trait]
pub trait SearchConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: &Params,
    ) -> Result<Headers, SearchError>;

    fn complete_url(&self, litellm_params: &Params) -> Result<String, SearchError>;

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError>;

    async fn atransform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        self.transform_search_request(request)
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, SearchError>;

    fn sign_request(&self, request: SignRequest<'_>) -> (Headers, Option<Vec<u8>>) {
        (request.headers.clone(), None)
    }

    // SignsRequestsWithAWS: this config's signing runs off the loop when the caller awaits
    fn signs_with_aws(&self) -> bool {
        false
    }

    // BaseQueryEmbeddingVectorStoreConfig reached through the config the caller holds
    fn query_embedding_config(&self) -> Option<&dyn QueryEmbeddingConfig> {
        None
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}

// The two members a query-embedding config adds to the search transform
#[async_trait]
pub trait QueryEmbeddingConfig: SearchConfig {
    fn transform_query_search(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError>;

    // LiteLLM's async default hands every argument, the executor included, to the sync transform once
    async fn atransform_query_search(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        self.transform_query_search(request)
    }
}

// which of LiteLLM's two embedding entry points the transform used: embed or aembedding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingEntry {
    Embedding,
    Aembedding,
}

#[async_trait]
pub trait QueryEmbeddingExecutor: Send + Sync {
    async fn embed(
        &self,
        entry: EmbeddingEntry,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, StoreFailure>;
}

// BaseDirectVectorStoreConfig.execute_search_vector_store_request's arguments
pub struct DirectSearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub logging: &'request mut VectorStoreLogging,
    pub litellm_params: &'request Params,
    pub embedding_executor: Option<&'request dyn QueryEmbeddingExecutor>,
    pub timeout: Option<Duration>,
}

// A direct store runs the search itself, so its own error travels to the caller unchanged
#[async_trait]
pub trait DirectSearchConfig: Send + Sync {
    fn execute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectFailure>;

    async fn aexecute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectFailure> {
        self.execute_search_request(request)
    }
}
