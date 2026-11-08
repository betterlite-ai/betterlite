use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::error::{BaseLlmException, ContainerError};
use super::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};

#[derive(Debug, Clone, Copy)]
pub struct ContainerRequestContext<'request> {
    pub litellm_params: &'request Map<String, Value>,
    pub headers: &'request BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ContainerListQuery<'request> {
    pub after: Option<&'request str>,
    pub limit: Option<i64>,
    pub order: Option<&'request str>,
    pub extra_query: Option<&'request BTreeMap<String, String>>,
}

// LiteLLM's BaseContainerConfig, with the logging object and httpx.Response replaced by the raw body
pub trait BaseContainerConfig: Send + Sync {
    fn get_supported_openai_params(&self) -> Vec<String>;

    fn map_openai_params(
        &self,
        // ast-grep-ignore: no-long-names
        container_create_optional_params: &Map<String, Value>,
        drop_params: bool,
    ) -> Map<String, Value>;

    fn validate_environment(
        &self,
        _headers: &BTreeMap<String, String>,
        _api_key: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Map<String, Value>,
    ) -> Result<String, ContainerError> {
        api_base
            .map(str::to_owned)
            .ok_or(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_request(
        &self,
        name: &str,
        // ast-grep-ignore: no-long-names
        container_create_optional_request_params: &Map<String, Value>,
        context: ContainerRequestContext<'_>,
    ) -> Map<String, Value>;

    // ast-grep-ignore: no-long-names
    fn transform_container_create_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerObject, ContainerError>;

    // ast-grep-ignore: no-long-names
    fn transform_container_list_request(
        &self,
        api_base: &str,
        context: ContainerRequestContext<'_>,
        query: &ContainerListQuery<'_>,
    ) -> (String, Map<String, Value>);

    // ast-grep-ignore: no-long-names
    fn transform_container_list_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerListResponse, ContainerError>;

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_request(
        &self,
        container_id: &str,
        api_base: &str,
        context: ContainerRequestContext<'_>,
    ) -> (String, Map<String, Value>);

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerObject, ContainerError>;

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_request(
        &self,
        container_id: &str,
        api_base: &str,
        context: ContainerRequestContext<'_>,
    ) -> (String, Map<String, Value>);

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_response(
        &self,
        raw_response: &[u8],
    ) -> Result<DeleteContainerResult, ContainerError>;

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_request(
        &self,
        container_id: &str,
        api_base: &str,
        context: ContainerRequestContext<'_>,
        query: &ContainerListQuery<'_>,
    ) -> (String, Map<String, Value>);

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_response(
        &self,
        raw_response: &[u8],
    ) -> Result<ContainerFileListResponse, ContainerError>;

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_request(
        &self,
        container_id: &str,
        file_id: &str,
        api_base: &str,
        context: ContainerRequestContext<'_>,
    ) -> (String, Map<String, Value>);

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_response(
        &self,
        raw_response: &[u8],
    ) -> Result<Vec<u8>, ContainerError>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &BTreeMap<String, String>,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }
    }
}
