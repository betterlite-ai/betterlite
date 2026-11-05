#![cfg(test)]

use betterlite_openai::api::vector_file_contracts::records::{
    VectorStoreFileAuthCredentials, VectorStoreFileContentResponse, VectorStoreFileCreateRequest,
    VectorStoreFileDeleteResponse, VectorStoreFileListQueryParams, VectorStoreFileListResponse,
    VectorStoreFileObject, VectorStoreFileUpdateRequest,
};
use rstest::rstest;
use secrecy::SecretString;

use super::config::{
    EndpointsByType, FileOperation, Headers, Params, PreparedRequest, SignRequest,
    VectorStoreFileResponse, VectorStoreFilesConfig,
};
use super::error::{BaseLlmException, VectorStoreFilesError};

struct MinimalConfig;

fn unsupported() -> VectorStoreFilesError {
    VectorStoreFilesError::Upstream(BaseLlmException {
        status_code: 501,
        message: "not implemented".to_owned(),
        headers: Params::new(),
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

fn params(text: &str) -> Params {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::create(FileOperation::Create)]
#[case::list(FileOperation::List)]
#[case::read(FileOperation::Read)]
#[case::update(FileOperation::Update)]
#[case::delete(FileOperation::Delete)]
fn supported_params_are_empty_for_every_operation(#[case] operation: FileOperation) {
    assert!(MinimalConfig.supported_params(operation).is_empty());
}

#[rstest]
#[case::drop_params(true)]
#[case::keep_params(false)]
fn map_params_returns_optional_params_unchanged(#[case] drop_params: bool) {
    let optional = params(r#"{"limit": 3, "extra": null}"#);
    let non_default = params(r#"{"limit": 3, "order": "asc"}"#);
    let mapped = MinimalConfig.map_params(
        FileOperation::List,
        &non_default,
        optional.clone(),
        drop_params,
    );
    assert_eq!(mapped, optional);
}

#[test]
fn validate_environment_returns_no_headers() {
    let incoming = Headers::from([("Authorization".to_owned(), "Bearer x".to_owned())]);
    assert!(
        MinimalConfig
            .validate_environment(&incoming, None)
            .is_empty()
    );
}

#[rstest]
#[case::explicit_empty(Some(""), "")]
#[case::supplied(Some("https://api.example.test/v1"), "https://api.example.test/v1")]
fn complete_url_returns_the_supplied_base(#[case] api_base: Option<&str>, #[case] expected: &str) {
    let url = MinimalConfig
        .complete_url(api_base, "vs_a", &Params::new())
        .unwrap();
    assert_eq!(url, expected);
}

#[test]
fn complete_url_requires_an_api_base() {
    let error = MinimalConfig
        .complete_url(None, "vs_a", &Params::new())
        .unwrap_err();
    assert_eq!(error, VectorStoreFilesError::ApiBaseRequired);
    assert_eq!(error.to_string(), "api_base is required");
}

#[test]
fn sign_request_returns_the_same_headers_and_no_body() {
    let headers = Headers::from([("X-Key".to_owned(), "v".to_owned())]);
    let (signed, body) = MinimalConfig.sign_request(SignRequest {
        headers: &headers,
        optional_params: &params(r#"{"a": 1}"#),
        request_data: &params(r#"{"b": 2}"#),
        api_base: "https://api.example.test",
        api_key: Some(&SecretString::from("key")),
    });
    assert_eq!(signed, headers);
    assert_eq!(body, None);
}

#[test]
fn error_class_retains_status_message_and_headers() {
    let headers = params(r#"{"Retry-After": 3}"#);
    let error = MinimalConfig.error_class("bad", 429, headers.clone());
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "bad");
    assert_eq!(error.headers, headers);
    assert_eq!(error.to_string(), "bad");
}
