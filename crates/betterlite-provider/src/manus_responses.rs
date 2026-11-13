use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_secret_managers::secret_lookup::SecretLookup;

use crate::api::responses_provider_contract::config::ResponsesProviderConfig;
use crate::api::responses_provider_contract::records::{
    CompactRequest, CreateRequest, Headers, IdRequest, InputItemsRequest, Params, PreparedRequest,
    ResponsesLogging, ResponsesUpstream,
};
use crate::compatible_providers::Compatible;
use crate::provider::Provider;

mod credentials;
pub mod error;
mod inherited;
mod json;
mod reply;
mod request;
mod url;

mod tests;

// LiteLLM defaults the base when neither the deployment nor the SDK sets one.
pub const MANUS_API_BASE: &str = "https://api.manus.im";
pub const MANUS_API_BASE_ENV: &str = "MANUS_API_BASE";
pub const MANUS_API_KEY_ENV: &str = "MANUS_API_KEY";
// Manus spells the auth header API_KEY, not Authorization: Bearer.
pub const API_KEY_HEADER: &str = "API_KEY";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";
pub const TASK_MODE: &str = "agent";
pub const UNKNOWN_ID_PREFIX: &str = "unknown-";
const V1_SUFFIX: &str = "/v1";
const RESPONSES_SUFFIX: &str = "/responses";
const AGENT_PROFILE_FIELD: &str = "agent_profile";
const EXTRA_BODY_FIELD: &str = "extra_body";

pub trait UnknownIdSource: Send + Sync {
    fn next_suffix(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UuidIdSource;

impl UnknownIdSource for UuidIdSource {
    fn next_suffix(&self) -> String {
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect()
    }
}

// Every outside value the config reads is passed in when it is built, so it never touches the process environment.
pub struct ManusResponsesConfig {
    pub globals: GlobalCredentials,
    pub secrets: SecretLookup,
    pub environment: BTreeMap<String, String>,
    pub clock: Box<dyn Fn() -> i64 + Send + Sync>,
    pub ids: Box<dyn UnknownIdSource>,
}

impl ManusResponsesConfig {
    #[must_use]
    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    #[must_use]
    pub fn unknown_id(&self) -> String {
        format!("{UNKNOWN_ID_PREFIX}{}", self.ids.next_suffix())
    }
}

#[async_trait]
impl ResponsesProviderConfig for ManusResponsesConfig {
    type Error = error::ManusResponsesError;

    fn provider(&self) -> Provider {
        Provider::Compatible(Compatible)
    }

    fn supports_native_websocket(&self) -> bool {
        false
    }

    fn should_fake_stream(
        &self,
        _model: Option<&str>,
        stream: Option<bool>,
        _custom_llm_provider: Option<&str>,
    ) -> bool {
        stream == Some(true)
    }

    fn supported_params(&self, _model: &str) -> Result<Vec<String>, Self::Error> {
        Ok(inherited::supported_params())
    }

    fn map_params(
        &self,
        optional_params: &ResponsesApiRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Result<Params, Self::Error> {
        inherited::map_params(optional_params)
    }

    fn validate_environment(
        &self,
        headers: &Headers,
        _model: &str,
        litellm_params: Option<&Params>,
    ) -> Result<Headers, Self::Error> {
        credentials::validated_headers(self, headers, litellm_params)
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        url::responses_url(self, api_base)
    }

    fn transform_create_request(&self, request: &CreateRequest) -> Result<Params, Self::Error> {
        request::create_body(request)
    }

    fn transform_create_response(
        &self,
        _model: &str,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error> {
        reply::transform(self, response, logging).map(|reply| reply.response)
    }

    fn transform_streaming_response(
        &self,
        _model: &str,
        _chunk: &Json,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesAPIStreamingResponse, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "streaming response",
        })
    }

    fn transform_get_request(&self, request: &IdRequest) -> Result<PreparedRequest, Self::Error> {
        request::get_request(request)
    }

    fn transform_get_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error> {
        reply::transform(self, response, logging).map(|reply| reply.response)
    }

    fn transform_delete_request(
        &self,
        _request: &IdRequest,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "delete request",
        })
    }

    fn transform_delete_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<DeleteResponseResult, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "delete response",
        })
    }

    fn transform_input_items_request(
        &self,
        _request: &InputItemsRequest,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "input items request",
        })
    }

    fn transform_input_items_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<Params, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "input items response",
        })
    }

    fn transform_cancel_request(
        &self,
        _request: &IdRequest,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "cancel request",
        })
    }

    fn transform_cancel_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "cancel response",
        })
    }

    fn transform_compact_request(
        &self,
        _request: &CompactRequest,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "compact request",
        })
    }

    fn transform_compact_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error> {
        Err(error::ManusResponsesError::UnsupportedOperation {
            operation: "compact response",
        })
    }
}
