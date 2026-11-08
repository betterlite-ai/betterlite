use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use betterlite_provider::api::vector_store_contracts::config::{
    AuthCredentials, EndpointsByType, Params, PreparedRequest, SearchRequest, VectorStoreConfig,
    VectorStoreLogging, VectorStoreResponse,
};

use super::error::ValkeyError;

pub struct ValkeyVectorStoreConfig;

impl VectorStoreConfig for ValkeyVectorStoreConfig {
    type Error = ValkeyError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        Ok(api_base.unwrap_or_default().to_owned())
    }

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(ValkeyError::DirectSearch)
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(ValkeyError::DirectSearch)
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(ValkeyError::SearchOnly)
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(ValkeyError::SearchOnly)
    }
}
