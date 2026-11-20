use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_http::interactions::base::{ErrorClasses, ResponseHeaders};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughSigning, PassthroughStreamCollector, PassthroughTarget,
    PassthroughUrlRequest,
};
use thiserror::Error;
use url::Url;

use super::auth;
use super::logging::NoSpendCollector;
use super::urls;

// LiteLLM's ValueError and provider exception text, kept apart so _handle_error can read the status
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error("{message}")]
    Message { status: u16, message: String },
    #[error("{message}")]
    ValueError { message: String },
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}

impl ConfigError {
    // an exception that carries status_code: _handle_error keeps that status and its origin
    #[must_use]
    pub const fn message(status: u16, message: String) -> Self {
        Self::Message { status, message }
    }

    // a Python ValueError or TypeError: _handle_error synthesizes 500 for it
    #[must_use]
    pub const fn value_error(message: String) -> Self {
        Self::ValueError { message }
    }

    #[must_use]
    pub const fn from_status(status: Option<u16>, message: String) -> Self {
        match status {
            Some(status) => Self::message(status, message),
            None => Self::value_error(message),
        }
    }
}

impl From<ProviderError> for ConfigError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}

// The error a step of the call raises before the send; _handle_error reads its status and text
pub(crate) struct FaultDetail {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
}

pub(crate) enum Fault {
    Failure(Box<FaultDetail>),
    Send(Box<TransportError>),
}

impl Fault {
    pub(crate) fn message(status: u16, synthesized: bool, message: String) -> Self {
        Self::Failure(Box::new(FaultDetail {
            status,
            synthesized,
            message,
        }))
    }
}

impl From<ConfigError> for Fault {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::Message { status, message } => Self::message(status, false, message),
            ConfigError::ValueError { message } => Self::message(500, true, message),
            ConfigError::Passthrough(error) => Self::message(500, true, error.to_string()),
            ConfigError::Provider(error) => Self::message(
                error.status_code,
                error.status_code_is_synthesized,
                error.message,
            ),
        }
    }
}

// The eight configs LiteLLM's pass-through factory returns, chosen by B5360's tag
pub struct PassthroughCallConfig {
    pub tag: PassthroughConfigTag,
}

impl PassthroughCallConfig {
    #[must_use]
    pub const fn new(tag: PassthroughConfigTag) -> Self {
        Self { tag }
    }
}

// LiteLLM: "stream" in endpoint for Bedrock, "stream" in request_data for VLLM, bool(request_data.get("stream"))
// for Azure and Azure AI, bool(request_data["stream"] is True) for GigaChat, Watsonx and NVIDIA NIM
pub(crate) fn streaming_request(
    tag: PassthroughConfigTag,
    endpoint: &str,
    request_data: &Object,
) -> bool {
    match tag {
        PassthroughConfigTag::Bedrock | PassthroughConfigTag::BedrockMantle => {
            endpoint.contains("stream")
        }
        PassthroughConfigTag::VLLM => request_data.contains_key("stream"),
        PassthroughConfigTag::Azure => request_data.get("stream").is_some_and(filled),
        PassthroughConfigTag::AzureAI => request_data.get("stream").is_some_and(truthy),
        PassthroughConfigTag::GigaChat
        | PassthroughConfigTag::Watsonx
        | PassthroughConfigTag::NvidiaNim => {
            matches!(request_data.get("stream"), Some(JsonValue::Bool(true)))
        }
    }
}

fn filled(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_none_or(|magnitude| magnitude != 0.0_f64),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}

pub(crate) fn truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null | JsonValue::Bool(false) => false,
        JsonValue::Bool(true) => true,
        JsonValue::Number(number) => number
            .as_f64()
            .is_some_and(|magnitude| magnitude != 0.0_f64),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}

impl BasePassthroughConfig for PassthroughCallConfig {
    type Error = ConfigError;
    type Logged = ();

    fn is_streaming_request(&self, endpoint: &str, request_data: &Object) -> bool {
        streaming_request(self.tag, endpoint, request_data)
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        urls::complete_url(self, request)
    }

    fn sign_request(
        &self,
        headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        auth::sign_request(self.tag, headers, context)
    }

    fn create_stream_collector(
        &self,
        _target: &PassthroughTarget<'_>,
    ) -> Box<dyn PassthroughStreamCollector<Logged = Self::Logged> + Send + '_>
    where
        Self: Sized + Sync,
    {
        Box::new(NoSpendCollector)
    }
}

// _handle_error maps a status failure to the provider's own error class through this trait
impl ErrorClasses for PassthroughCallConfig {
    type Error = ConfigError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> Self::Error {
        let parts = ProviderErrorParts {
            status_code,
            message: message.to_owned(),
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
        };
        ConfigError::from(ProviderError::from(parts))
    }
}
