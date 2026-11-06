use std::collections::BTreeMap;
use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreOpenAiParam;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, VectorStoreCreateResponse, VectorStoreIndexEndpoints,
    VectorStoreSearchResponse,
};
use betterlite_openai::embedding::EmbeddingResponse;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::error::{BaseLlmException, VectorStoreContractError, VectorStoreEmbeddingError};

pub type Params = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, String>;
pub type PreparedRequest = (String, Params);
pub type AuthCredentials = BaseVectorStoreAuthCredentials;
pub type EndpointsByType = VectorStoreIndexEndpoints;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchQuery {
    Text(String),
    Sequence(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreLogging {
    pub model_call_details: Params,
}

pub struct SearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub api_base: &'request str,
    pub logging: &'request mut VectorStoreLogging,
    pub litellm_params: &'request Params,
    pub extra_body: Option<&'request Params>,
}

#[derive(Debug, Clone, Copy)]
pub struct SignRequest<'request> {
    pub headers: &'request Headers,
    pub optional_params: &'request Params,
    pub request_data: &'request Params,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorStoreResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

// LiteLLM's BaseVectorStoreConfig: providers fill the required members and the rest keep the base behavior
#[async_trait]
pub trait VectorStoreConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<VectorStoreContractError>;

    fn validate_create_vector_store(&self) {}

    fn supported_params(&self, _model: &str) -> Vec<VectorStoreOpenAiParam> {
        Vec::new()
    }

    fn map_params(
        &self,
        _non_default_params: &Params,
        optional_params: Params,
        _drop_params: bool,
    ) -> Params {
        optional_params
    }

    fn auth_credentials(&self, litellm_params: &Params) -> AuthCredentials;

    fn endpoints_by_type(&self) -> EndpointsByType;

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    async fn transform_search_request_async(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        self.transform_search_request(request)
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error>;

    fn transform_create_request(
        &self,
        optional_params: &Params,
        api_base: &str,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_create_response(
        &self,
        response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error>;

    fn validate_environment(
        &self,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Headers {
        Headers::new()
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        api_base
            .map(str::to_owned)
            .ok_or_else(|| Self::Error::from(VectorStoreContractError::ApiBaseRequired))
    }

    fn error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: Params,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers,
        }
    }

    fn sign_request(&self, request: SignRequest<'_>) -> (Headers, Option<Vec<u8>>) {
        (request.headers.clone(), None)
    }

    fn calculate_vector_store_cost(
        &self,
        _response: &VectorStoreSearchResponse,
    ) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }
}

#[async_trait]
pub trait VectorStoreEmbeddingExecutor: Send + Sync {
    async fn embed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError>;

    // betterlite has one async execution, so LiteLLM's aembed is the same call
    async fn aembed(
        &self,
        model: &str,
        query: &str,
        configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        self.embed(model, query, configuration).await
    }
}
