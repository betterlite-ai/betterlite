use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::FileContentStreamingResult;
use futures_util::stream::BoxStream;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::BaseLlmException;

use super::error::ContentError;

pub type Headers = BTreeMap<String, String>;
// LiteLLM's file content transform returns dict[str, str], and its order is the order a query string appends in
pub type QueryParams = IndexMap<String, String>;
pub type BodyStream = BoxStream<'static, Result<Vec<u8>, HttpError>>;

// The litellm_params fields the content handler reads
#[derive(Debug, Clone, Default)]
pub struct ContentParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

// httpx.Response as a provider's response transform reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferedResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub url: String,
}

pub struct StreamContext<'call> {
    pub request_url: &'call str,
    pub logging: &'call mut LoggingState,
    pub params: &'call ContentParams,
}

// The BaseFilesConfig members the content handler calls; B1030 adapts a provider config to this
pub trait FileContentConfig: Send + Sync {
    fn transform_content_request(
        &self,
        request: &FileContentRequest,
        params: &ContentParams,
    ) -> Result<(String, QueryParams), ContentError>;

    fn validate_environment(
        &self,
        headers: Headers,
        params: &ContentParams,
    ) -> Result<Headers, ContentError>;

    fn transform_content_response(
        &self,
        response: &BufferedResponse,
        logging: &mut LoggingState,
        params: &ContentParams,
    ) -> Result<BinaryContent, ContentError>;

    fn transform_content_stream(
        &self,
        body: BodyStream,
        headers: &HeaderMap,
        chunk_size: NonZeroUsize,
        _context: StreamContext<'_>,
    ) -> FileContentStreamingResult {
        FileContentStreamingResult::new(body, headers, Some(chunk_size))
    }

    fn error_class(&self, message: &str, status_code: u16, headers: Headers) -> BaseLlmException;
}
