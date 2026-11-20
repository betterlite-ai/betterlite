use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::{RawResponse, ResponsesReply};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::responses_compact_http::error::ConfigFailure;
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};

use super::convert::ValueKind as _;
use super::error::ParamError;

pub type Headers = BTreeMap<String, String>;
pub type Params = IndexMap<String, Json>;

pub const API_BASE_KEY: &str = "api_base";
pub const API_KEY_KEY: &str = "api_key";
pub const SSL_VERIFY_KEY: &str = "ssl_verify";

// The litellm_params fields both create handlers read directly, plus every other key dict(litellm_params) carries
#[derive(Debug, Clone, Default)]
pub struct CreateParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
    pub extra: Params,
}

impl CreateParams {
    // dict(litellm_params) as sign_request, the agentic hooks and request_context receive it
    #[must_use]
    pub fn to_map(&self) -> Params {
        let mut map = self.extra.clone();
        if let Some(api_base) = &self.api_base {
            map.insert(API_BASE_KEY.to_owned(), Json::Text(api_base.clone()));
        }
        if let Some(api_key) = &self.api_key {
            map.insert(
                API_KEY_KEY.to_owned(),
                Json::Text(api_key.expose_secret().to_owned()),
            );
        }
        if let Some(ssl_verify) = self.ssl_verify {
            map.insert(SSL_VERIFY_KEY.to_owned(), Json::Bool(ssl_verify));
        }
        map
    }

    pub fn from_map(map: Params) -> Result<Self, ParamError> {
        let mut params = Self::default();
        for (key, value) in map {
            let name = key.as_str();
            match (name, value) {
                (API_BASE_KEY | API_KEY_KEY | SSL_VERIFY_KEY, Json::Null) => {}
                (API_BASE_KEY, Json::Text(text)) => params.api_base = Some(text),
                (API_KEY_KEY, Json::Text(text)) => params.api_key = Some(SecretString::from(text)),
                (SSL_VERIFY_KEY, Json::Bool(flag)) => params.ssl_verify = Some(flag),
                (API_BASE_KEY | API_KEY_KEY, other) => {
                    return Err(ParamError::FieldType {
                        field: name.to_owned(),
                        expected: "a string",
                        found: other.value_kind(),
                    });
                }
                (SSL_VERIFY_KEY, other) => {
                    return Err(ParamError::FieldType {
                        field: name.to_owned(),
                        expected: "a boolean",
                        found: other.value_kind(),
                    });
                }
                (_, other) => {
                    params.extra.insert(name.to_owned(), other);
                }
            }
        }
        Ok(params)
    }
}

// The arguments of transform_responses_api_request, in the order LiteLLM passes them
#[derive(Debug, Clone, Copy)]
pub struct CreateTransformRequest<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub optional_params: &'call Object,
    pub params: &'call CreateParams,
    pub headers: &'call Headers,
}

// The arguments of sign_request once the body is final, plus the post-fake-prep stream flags
#[derive(Debug, Clone, Copy)]
pub struct CreateSignRequest<'call> {
    pub headers: &'call Headers,
    pub optional_params: &'call Params,
    pub request_data: &'call Object,
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub model: &'call str,
    pub stream: bool,
    pub fake_stream: Option<&'call Json>,
}

// The arguments of transform_extra_body, whose base implementation returns the body as given
#[derive(Debug, Clone, Copy)]
pub struct ExtraBodyRequest<'call> {
    pub extra_body: &'call Object,
    pub request: &'call Object,
    pub model: &'call str,
    pub params: &'call CreateParams,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
}

// The BaseResponsesAPIConfig members both create handlers call; B1114 adapts a provider config to this
pub trait CreateConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: Headers,
        model: &str,
        params: &CreateParams,
    ) -> Result<Headers, ConfigFailure>;

    fn complete_url(&self, params: &CreateParams) -> Result<String, ConfigFailure>;

    fn transform_request(
        &self,
        request: &CreateTransformRequest<'_>,
    ) -> Result<Object, ConfigFailure>;

    // The async handler awaits async_transform_responses_api_request, which only some providers override
    fn transform_request_off_loop(
        &self,
        request: &CreateTransformRequest<'_>,
    ) -> impl Future<Output = Result<Object, ConfigFailure>> + Send {
        ready(self.transform_request(request))
    }

    // BaseResponsesAPIConfig.normalize_responses_api_request_dict; B1114 supplies the production one
    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure>;

    fn transform_extra_body(
        &self,
        request: &ExtraBodyRequest<'_>,
    ) -> Result<Object, ConfigFailure> {
        Ok(request.extra_body.clone())
    }

    fn sign_request(&self, request: &CreateSignRequest<'_>) -> Result<Signed, ConfigFailure>;

    // LiteLLM awaits sign_request_off_loop_if_aws: only AWS signing leaves the loop
    fn sign_request_off_loop(
        &self,
        request: &CreateSignRequest<'_>,
    ) -> impl Future<Output = Result<Signed, ConfigFailure>> + Send {
        ready(self.sign_request(request))
    }

    fn transform_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}
