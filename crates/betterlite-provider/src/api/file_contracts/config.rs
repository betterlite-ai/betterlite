use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::file_type_records::{
    ByteStream, FileContentStreamingResult, StreamingMediaUploadConfig, TwoStepFileUploadConfig,
};
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileObject,
};
use betterlite_openai::openai_base_config::Object;
use http::Method;
use secrecy::SecretString;

use super::base::BaseConfig;
use super::error::FileContractError;

pub type FileParams = Object;
pub type Headers = BTreeMap<String, String>;
pub type PreparedRequest = (String, FileParams);
pub type NextPageRequest = (String, BTreeMap<String, String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpenAiParam {
    Purpose,
}

// httpx.Response, with the body kept as bytes and the request URL optional for the transform that reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub url: Option<String>,
}

// The fields a provider transform reads from litellm's logging object; the pass-through defaults ignore it
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileLogging {
    pub model_call_details: FileParams,
}

// LiteLLM's Union[dict, str, bytes, TwoStepFileUploadConfig], plus the streaming media plan its dict branch carries
pub enum CreateFileBody {
    Object(FileParams),
    Text(String),
    Bytes(Vec<u8>),
    TwoStep(Box<TwoStepFileUploadConfig>),
    StreamingMedia(Box<StreamingMediaUploadConfig>),
}

pub struct CompleteUrlRequest<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub model: &'request str,
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
}

pub struct FileUrlRequest<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub model: &'request str,
    pub optional_params: &'request FileParams,
    pub litellm_params: &'request FileParams,
    pub data: &'request FileCreate,
}

pub struct StreamContext<'request> {
    pub request_url: &'request str,
    pub logging_obj: &'request mut FileLogging,
    pub litellm_params: &'request FileParams,
}

pub trait BaseFilesConfig: BaseConfig + Send + Sync {
    fn custom_llm_provider(&self) -> ProviderId;

    fn file_upload_http_method(&self) -> Method {
        Method::POST
    }

    fn get_supported_openai_params(&self, model: &str) -> Vec<FileOpenAiParam>;

    fn get_complete_file_url(
        &self,
        request: FileUrlRequest<'_>,
    ) -> Result<String, FileContractError> {
        self.get_complete_url(CompleteUrlRequest {
            api_base: request.api_base,
            api_key: request.api_key,
            model: request.model,
            optional_params: request.optional_params,
            litellm_params: request.litellm_params,
        })
    }

    fn transform_create_file_request(
        &self,
        model: &str,
        create_file_data: &FileCreate,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<CreateFileBody, FileContractError>;

    fn transform_create_file_response(
        &self,
        model: Option<&str>,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<FileObject, FileContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_request(
        &self,
        file_id: &str,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<PreparedRequest, FileContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_file_response(
        &self,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<FileObject, FileContractError>;

    fn transform_delete_file_request(
        &self,
        file_id: &str,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<PreparedRequest, FileContractError>;

    fn transform_delete_file_response(
        &self,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<FileDeleted, FileContractError>;

    fn transform_list_files_request(
        &self,
        purpose: Option<&str>,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<PreparedRequest, FileContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_list_files_next_request(
        &self,
        _raw_response: &FileResponse,
        _optional_params: &FileParams,
        _litellm_params: &FileParams,
    ) -> Option<NextPageRequest> {
        None
    }

    fn transform_list_files_response(
        &self,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<Vec<FileObject>, FileContractError>;

    fn transform_file_content_request(
        &self,
        file_content_request: &FileContentRequest,
        optional_params: &FileParams,
        litellm_params: &FileParams,
    ) -> Result<PreparedRequest, FileContractError>;

    // ast-grep-ignore: no-long-names
    fn transform_file_content_response(
        &self,
        raw_response: &FileResponse,
        logging_obj: &mut FileLogging,
        litellm_params: &FileParams,
    ) -> Result<BinaryContent, FileContractError>;

    fn transform_file_content_stream(
        &self,
        stream_iterator: ByteStream,
        headers: Headers,
        _context: StreamContext<'_>,
    ) -> FileContentStreamingResult {
        FileContentStreamingResult {
            stream_iterator,
            headers,
        }
    }
}
