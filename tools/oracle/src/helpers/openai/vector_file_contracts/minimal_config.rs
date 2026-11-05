use betterlite_openai::api::vector_file_contracts::records::{
    VectorStoreFileAuthCredentials, VectorStoreFileContentResponse, VectorStoreFileCreateRequest,
    VectorStoreFileDeleteResponse, VectorStoreFileListQueryParams, VectorStoreFileListResponse,
    VectorStoreFileObject, VectorStoreFileUpdateRequest,
};
use betterlite_provider::api::vector_file_contracts::config::{
    EndpointsByType, Params, PreparedRequest, VectorStoreFileResponse, VectorStoreFilesConfig,
};
use betterlite_provider::api::vector_file_contracts::error::{
    BaseLlmException, VectorStoreFilesError,
};
use serde_json::Map;

pub(super) struct MinimalConfig;

fn unsupported() -> VectorStoreFilesError {
    VectorStoreFilesError::Upstream(BaseLlmException {
        status_code: 501,
        message: "not implemented".to_owned(),
        headers: Map::new(),
    })
}

impl VectorStoreFilesConfig for MinimalConfig {
    fn auth_credentials(&self, _litellm_params: &Params) -> VectorStoreFileAuthCredentials {
        VectorStoreFileAuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::new()
    }

    fn transform_create_request(
        &self,
        _vector_store_id: &str,
        _create_request: &VectorStoreFileCreateRequest,
        _api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileObject, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_list_request(
        &self,
        _vector_store_id: &str,
        _query_params: &VectorStoreFileListQueryParams,
        _api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_list_response(
        &self,
        _response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileListResponse, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_retrieve_request(
        &self,
        _vector_store_id: &str,
        _file_id: &str,
        _api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_retrieve_response(
        &self,
        _response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileObject, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_content_request(
        &self,
        _vector_store_id: &str,
        _file_id: &str,
        _api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_content_response(
        &self,
        _response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileContentResponse, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_update_request(
        &self,
        _vector_store_id: &str,
        _file_id: &str,
        _update_request: &VectorStoreFileUpdateRequest,
        _api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_update_response(
        &self,
        _response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileObject, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_delete_request(
        &self,
        _vector_store_id: &str,
        _file_id: &str,
        _api_base: &str,
    ) -> Result<PreparedRequest, VectorStoreFilesError> {
        Err(unsupported())
    }

    fn transform_delete_response(
        &self,
        _response: &VectorStoreFileResponse,
    ) -> Result<VectorStoreFileDeleteResponse, VectorStoreFilesError> {
        Err(unsupported())
    }
}
