use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::BoxError;
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::VideoObject;

pub type Params = IndexMap<String, Json>;
pub type Headers = IndexMap<String, String>;

// One httpx RequestFiles entry: the form field, an optional filename and content type, and the bytes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestFile {
    pub field: String,
    pub filename: Option<String>,
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}

pub type RequestFiles = Vec<RequestFile>;

// The part of LiteLLM's logging object a provider transform reads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoLogging {
    pub model_call_details: Params,
}

// api_base, litellm_params and headers are the last three arguments of every video request transform
#[derive(Debug, Clone, Copy)]
pub struct VideoRequestContext<'request> {
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

#[derive(Debug)]
pub struct VideoReply<'reply> {
    pub raw_response: &'reply UpstreamResponse,
    pub logging_obj: &'reply mut VideoLogging,
    pub custom_llm_provider: Option<&'reply str>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoEnvironment<'request> {
    pub headers: &'request Headers,
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub litellm_params: Option<&'request Params>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoEditInput<'request> {
    pub prompt: &'request str,
    pub video_id: &'request str,
    pub video_file: Option<&'request SuppliedMedia>,
    pub extra_body: Option<&'request Params>,
    pub prefetched_source_data: Option<&'request Params>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoExtensionInput<'request> {
    pub prompt: &'request str,
    pub video_id: &'request str,
    pub seconds: &'request str,
    pub extra_body: Option<&'request Params>,
}

// B3331's trait of this name lives in betterlite-provider, which depends on this crate, so its members are mirrored here
pub trait BaseVideoConfig: Send + Sync {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers;

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, BoxError>;

    // None means the edit needs no pre-fetch call, and Some is the (url, body) to send first
    fn get_video_edit_prefetch_params(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Option<(String, Params)>;

    fn transform_video_edit_request(
        &self,
        input: VideoEditInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params, Option<RequestFiles>), BoxError>;

    fn transform_video_edit_response(
        &self,
        reply: VideoReply<'_>,
        request_data: Option<&Params>,
    ) -> Result<VideoObject, BoxError>;

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_request(
        &self,
        input: VideoExtensionInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), BoxError>;

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, BoxError>;

    // B2202's BaseLLMException with the three supplied facts; LiteLLM raises it where this returns it
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError;
}
