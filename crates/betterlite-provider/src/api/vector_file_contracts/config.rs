use std::collections::BTreeMap;

use betterlite_openai::api::vector_file_contracts::records::{
    FileParams, VectorStoreFileAuthCredentials, VectorStoreFileContentResponse,
    VectorStoreFileCreateRequest, VectorStoreFileDeleteResponse, VectorStoreFileListQueryParams,
    VectorStoreFileListResponse, VectorStoreFileObject, VectorStoreFileUpdateRequest,
};
use secrecy::SecretString;

use super::error::{BaseLlmException, VectorStoreFilesError};

pub type Params = FileParams;
pub type Headers = BTreeMap<String, String>;
pub type EndpointsByType = BTreeMap<String, Vec<(String, String)>>;
pub type PreparedRequest = (String, Params);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOperation {
    Create,
    List,
    Read,
    Update,
    Delete,
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
pub struct VectorStoreFileResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportedParams(pub &'static [&'static str]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentHeaders(pub Headers);

pub trait VectorStoreFilesConfig {
    fn supported_params(&self, _operation: FileOperation) -> SupportedParams {
        SupportedParams(&[])
    }

    fn map_params(
        &self,
        _operation: FileOperation,
        _non_default_params: &Params,
        optional_params: Params,
        _drop_params: bool,
    ) -> Params {
        optional_params
    }

    fn auth_credentials(&self, litellm_params: &Params) -> VectorStoreFileAuthCredentials;

    fn endpoints_by_type(&self) -> EndpointsByType;

    fn validate_environment(
        &self,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> EnvironmentHeaders {
        EnvironmentHeaders(Headers::new())
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _vector_store_id: &str,
        _litellm_params: &Params,
    ) -> Result<String, VectorStoreFilesError> {
        api_base
            .map(str::to_owned)
            .ok_or(VectorStoreFilesError::ApiBaseRequired)
    }

    fn transform_create_request(
        &self,
        vector_store_id: &str,
        create_request: &VectorStoreFileCreateRequest,
        api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError>;

    fn transform_create_response(
        &self,
        response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileObject, VectorStoreFilesError>;

    fn transform_list_request(
        &self,
        vector_store_id: &str,
        query_params: &VectorStoreFileListQueryParams,
        api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError>;

    fn transform_list_response(
        &self,
        response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileListResponse, VectorStoreFilesError>;

    fn transform_retrieve_request(
        &self,
        vector_store_id: &str,
        file_id: &str,
        api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError>;

    fn transform_retrieve_response(
        &self,
        response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileObject, VectorStoreFilesError>;

    fn transform_content_request(
        &self,
        vector_store_id: &str,
        file_id: &str,
        api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError>;

    fn transform_content_response(
        &self,
        response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileContentResponse, VectorStoreFilesError>;

    fn transform_update_request(
        &self,
        vector_store_id: &str,
        file_id: &str,
        update_request: &VectorStoreFileUpdateRequest,
        api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError>;

    fn transform_update_response(
        &self,
        response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileObject, VectorStoreFilesError>;

    fn transform_delete_request(
        &self,
        vector_store_id: &str,
        file_id: &str,
        api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError>;

    fn transform_delete_response(
        &self,
        response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileDeleteResponse, VectorStoreFilesError>;

    // LiteLLM raises this exception; returning it lets the caller decide where to fail.
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
}
