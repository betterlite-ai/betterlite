use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::containers::container_request_parameters::ContainerFieldTypeError;

#[derive(Debug, Error)]
pub enum ContainerFileOpError {
    #[error("Unknown endpoint: {name}")]
    UnknownEndpoint { name: String },
    #[error("the mock response is not valid JSON")]
    MockJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match {response_type}")]
    MockModel {
        response_type: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Decode(#[from] ContainerFieldTypeError),
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    #[error("Container provider config not found for: {provider}")]
    NoProviderConfig { provider: String },
    #[error("the default request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] ContainerFileHttpError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct ContainerFileFailureError {
    #[source]
    pub error: ContainerFileOpError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...), run once
#[must_use]
pub fn failure(
    error: ContainerFileOpError,
    provider: &str,
    redactor: &Redactor,
) -> ContainerFileFailureError {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(exception(&error, provider))),
        redactor,
    );
    ContainerFileFailureError { error, class }
}

// a provider exception keeps its status; everything else reaches exception_type's catchall
fn exception(error: &ContainerFileOpError, provider: &str) -> RawException {
    if let Some(provider_error) = upstream(error) {
        return provider_status(provider_error, provider);
    }
    value_error(error, provider)
}

// a handler-level and a config-level upstream exception both keep their status
const fn upstream(error: &ContainerFileOpError) -> Option<&BaseLlmException> {
    let ContainerFileOpError::Handler(handler_error) = error else {
        return None;
    };
    if let ContainerFileHttpError::Provider(provider_error) = handler_error {
        return Some(provider_error);
    }
    if let ContainerFileHttpError::Config(ContainerError::Upstream(provider_error)) = handler_error
    {
        return Some(provider_error);
    }
    None
}

fn provider_status(upstream: &BaseLlmException, provider: &str) -> RawException {
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: upstream.message.clone(),
        status_code: Some(upstream.status_code),
        status_code_is_synthesized: false,
        message: Some(upstream.message.clone()),
        body: None,
        exception_type: String::new(),
    }
}

fn value_error(error: &ContainerFileOpError, provider: &str) -> RawException {
    let message = error.to_string();
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: "ValueError".to_owned(),
    }
}
