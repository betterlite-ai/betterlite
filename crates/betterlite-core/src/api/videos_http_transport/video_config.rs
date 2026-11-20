use std::error::Error as StdError;
use std::future::{Future, ready};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use indexmap::IndexMap;
use secrecy::SecretString;

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

// The three facts _handle_error reads off an exception that is not an HTTP status error
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorFacts {
    pub message: String,
    pub status_code: Option<u16>,
    pub headers: Headers,
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

// LiteLLM's BaseVideoConfig members the generation and content handlers call; a provider adapter implements this over its own config
pub trait VideoHttpConfig: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    // The base body is the empty mapping, and a provider starts from the extra headers it was handed
    #[cfg_attr(test, mutants::skip)]
    fn validate_environment(&self, _request: VideoEnvironment<'_>) -> Headers {
        Headers::new()
    }

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, Self::Error>;

    fn use_multipart_form_data(&self) -> bool {
        false
    }

    fn transform_video_create_request(
        &self,
        model: &str,
        prompt: &str,
        optional_params: &Params,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_create_response(
        &self,
        model: &str,
        reply: VideoReply<'_>,
        request_data: Option<&Params>,
    ) -> Result<VideoObject, Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_content_request(
        &self,
        video_id: &str,
        variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, Self::Error>;

    // ast-grep-ignore: no-long-names
    fn async_transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(self.transform_video_content_response(raw_response, logging_obj))
    }

    // _handle_error's getattr chain: a plain exception has no status, and one that does keeps it
    fn error_facts(&self, error: &Self::Error) -> ErrorFacts {
        ErrorFacts {
            message: error.to_string(),
            status_code: None,
            headers: Headers::new(),
        }
    }

    // BaseVideoConfig.get_error_class raises the provider's error class with the three facts it was handed
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
