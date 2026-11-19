use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::response_events::{RawResponse, ResponsesReply};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_resources::request::ResourceRequest;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};

use super::error::ConfigFailure;

pub type Headers = BTreeMap<String, String>;

// The litellm_params fields both compact handlers read before handing the rest to the provider config
#[derive(Debug, Clone, Default)]
pub struct CompactParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The arguments of transform_compact_response_api_request, in the order LiteLLM passes them
#[derive(Debug, Clone, Copy)]
pub struct CompactTransformRequest<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub optional_params: &'call Object,
    pub api_base: &'call str,
    pub params: &'call CompactParams,
    pub headers: &'call Headers,
}

// The arguments of sign_request after normalization, plus the final compact URL
#[derive(Debug, Clone, Copy)]
pub struct SignRequest<'call> {
    pub headers: &'call Headers,
    pub optional_params: &'call CompactParams,
    pub request_data: &'call Object,
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub model: Option<&'call str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
}

// The BaseResponsesAPIConfig members the two compact handlers call; B934 adapts a provider config to this
pub trait CompactConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: Headers,
        model: &str,
        params: &CompactParams,
    ) -> Result<Headers, ConfigFailure>;

    fn complete_url(&self, params: &CompactParams) -> Result<String, ConfigFailure>;

    fn transform_compact_request(
        &self,
        request: &CompactTransformRequest<'_>,
    ) -> Result<ResourceRequest, ConfigFailure>;

    // BaseResponsesAPIConfig.normalize_responses_api_request_dict; B1114 supplies the production one
    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure>;

    fn sign_request(&self, request: &SignRequest<'_>) -> Result<Signed, ConfigFailure>;

    // LiteLLM awaits sign_request_off_loop_if_aws: the AWS signing protocol overrides this to leave the loop,
    // and every other config keeps the default, which signs on the caller's thread
    fn sign_request_off_loop(
        &self,
        request: &SignRequest<'_>,
    ) -> impl Future<Output = Result<Signed, ConfigFailure>> + Send {
        ready(self.sign_request(request))
    }

    fn transform_compact_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}
