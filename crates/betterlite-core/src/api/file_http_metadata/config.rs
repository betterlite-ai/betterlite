use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{FileDeleted, FileObject};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};

pub type FileParams = Object;
pub type Headers = BTreeMap<String, String>;
pub type PreparedRequest = (String, FileParams);
pub type NextPageRequest = (String, BTreeMap<String, String>);

// httpx.Response, with the body kept as bytes and the request URL a provider transform reads for the file id
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub url: Option<String>,
}

// What a response transform reads: the logging object's model_call_details, borrowed so the transform's writes stick, and the request's litellm_params
pub struct ReplyContext<'call> {
    pub model_call_details: &'call mut IndexMap<String, Json>,
    pub litellm_params: &'call FileParams,
}

// The six arguments validate_environment receives, all of them empty except the headers, the key and litellm_params
pub struct EnvironmentRequest<'request> {
    pub api_key: Option<&'request SecretString>,
    pub headers: Headers,
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
}

pub struct FileRequest<'request> {
    pub file_id: &'request str,
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
}

pub struct ListRequest<'request> {
    pub purpose: Option<&'request str>,
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
}

// B1015's file contract over the members these handlers call; B1028 and B1029 adapt a provider config to it
pub trait MetadataConfig: Send + Sync {
    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, BaseLlmException>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_response(
        &self,
        raw_response: &FileResponse,
        context: ReplyContext<'_>,
    ) -> Result<FileObject, BaseLlmException>;

    fn transform_delete_file_request(
        &self,
        request: FileRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException>;

    fn transform_delete_file_response(
        &self,
        raw_response: &FileResponse,
        context: ReplyContext<'_>,
    ) -> Result<FileDeleted, BaseLlmException>;

    fn transform_list_files_request(
        &self,
        request: ListRequest<'_>,
    ) -> Result<PreparedRequest, BaseLlmException>;

    // ast-grep-ignore: no-long-names
    fn transform_list_files_next_request(
        &self,
        raw_response: &FileResponse,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Option<NextPageRequest>;

    fn transform_list_files_response(
        &self,
        raw_response: &FileResponse,
        context: ReplyContext<'_>,
    ) -> Result<Vec<FileObject>, BaseLlmException>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}
