use betterlite_openai::api::vector_file_contracts::records::{
    VectorStoreFileAuthCredentials, VectorStoreFileContentResponse,
    VectorStoreFileCreateRequest as CreateBody, VectorStoreFileDeleteResponse,
    VectorStoreFileListQueryParams as ListQuery, VectorStoreFileListResponse,
    VectorStoreFileObject, VectorStoreFileUpdateRequest as UpdateBody,
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

type Prepared = Result<PreparedRequest, VectorStoreFilesError>;
type Reply<T> = Result<T, VectorStoreFilesError>;

impl VectorStoreFilesConfig for MinimalConfig {
    fn auth_credentials(&self, _litellm_params: &Params) -> VectorStoreFileAuthCredentials {
        VectorStoreFileAuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::new()
    }

    fn transform_create_request(&self, _: &str, _: &CreateBody, _: &str) -> Prepared {
        Err(unsupported())
    }

    fn transform_create_response(
        &self,
        _: &VectorStoreFileResponse,
    ) -> Reply<VectorStoreFileObject> {
        Err(unsupported())
    }

    fn transform_list_request(&self, _: &str, _: &ListQuery, _: &str) -> Prepared {
        Err(unsupported())
    }

    fn transform_list_response(
        &self,
        _: &VectorStoreFileResponse,
    ) -> Reply<VectorStoreFileListResponse> {
        Err(unsupported())
    }

    fn transform_retrieve_request(&self, _: &str, _: &str, _: &str) -> Prepared {
        Err(unsupported())
    }

    fn transform_retrieve_response(
        &self,
        _: &VectorStoreFileResponse,
    ) -> Reply<VectorStoreFileObject> {
        Err(unsupported())
    }

    fn transform_content_request(&self, _: &str, _: &str, _: &str) -> Prepared {
        Err(unsupported())
    }

    fn transform_content_response(
        &self,
        _: &VectorStoreFileResponse,
    ) -> Reply<VectorStoreFileContentResponse> {
        Err(unsupported())
    }

    fn transform_update_request(&self, _: &str, _: &str, _: &UpdateBody, _: &str) -> Prepared {
        Err(unsupported())
    }

    fn transform_update_response(
        &self,
        _: &VectorStoreFileResponse,
    ) -> Reply<VectorStoreFileObject> {
        Err(unsupported())
    }

    fn transform_delete_request(&self, _: &str, _: &str, _: &str) -> Prepared {
        Err(unsupported())
    }

    fn transform_delete_response(
        &self,
        _: &VectorStoreFileResponse,
    ) -> Reply<VectorStoreFileDeleteResponse> {
        Err(unsupported())
    }
}
