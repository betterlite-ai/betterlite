use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreCreateResponse, VectorStoreIndexEndpoints,
    VectorStoreSearchResponse,
};

use crate::api::vector_store_contracts::config::{
    AuthCredentials, EndpointsByType, Params, PreparedRequest, SearchQuery, SearchRequest,
    VectorStoreConfig, VectorStoreEmbeddingExecutor, VectorStoreLogging, VectorStoreResponse,
};

use super::error::DirectStoreError;

// LiteLLM's direct stores run the search on their datastore instead of building an HTTP request
pub struct DirectSearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub logging: &'request mut VectorStoreLogging,
    pub litellm_params: &'request Params,
    pub embedding_executor: Option<&'request dyn VectorStoreEmbeddingExecutor>,
    pub timeout: Option<Duration>,
}

pub const NO_REQUEST_SHAPE: &str =
    "Direct vector store providers execute the search themselves; no HTTP request shape";
pub const NO_RESPONSE_SHAPE: &str =
    "Direct vector store providers execute the search themselves; no HTTP response shape";

#[async_trait]
pub trait DirectVectorStoreConfig: Send + Sync {
    fn execute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectStoreError>;

    // a datastore driver that needs asynchronous I/O overrides this; the default runs the sync search
    async fn aexecute_search_request(
        &self,
        request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectStoreError> {
        self.execute_search_request(request)
    }
}

// BaseDirectVectorStoreConfig inherits the HTTP transforms only to reject them and serves its own search
#[async_trait]
impl<T: DirectVectorStoreConfig> VectorStoreConfig for T {
    type Error = DirectStoreError;

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, DirectStoreError> {
        Err(DirectStoreError::NotImplemented {
            message: NO_REQUEST_SHAPE,
        })
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, DirectStoreError> {
        Err(DirectStoreError::NotImplemented {
            message: NO_RESPONSE_SHAPE,
        })
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, DirectStoreError> {
        Err(DirectStoreError::NotImplemented { message: "" })
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, DirectStoreError> {
        Err(DirectStoreError::NotImplemented { message: "" })
    }

    // LiteLLM's direct base keeps a null api_base as the empty string
    fn complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, DirectStoreError> {
        Ok(api_base.unwrap_or_default().to_owned())
    }

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        BaseVectorStoreAuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        VectorStoreIndexEndpoints {
            read: Some(Vec::new()),
            write: Some(Vec::new()),
            extra: BTreeMap::new(),
        }
    }
}
