use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_openai::containers::container_file_http::transport::TransferError;
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
    route: Option<CallRoute<'_>>,
) -> ContainerFileFailureError {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(exception(&error, provider, route))),
        redactor,
    );
    ContainerFileFailureError { error, class }
}

// the method and handler form that sent the call, which decides whether a timeout was caught
#[derive(Clone, Copy)]
pub struct CallRoute<'route> {
    pub method: &'route str,
    pub asynchronous: bool,
}

// a provider exception keeps its status, a caught transport timeout is a 408, and everything else reaches exception_type's catchall
fn exception(
    error: &ContainerFileOpError,
    provider: &str,
    route: Option<CallRoute<'_>>,
) -> RawException {
    if let Some(provider_error) = upstream(error) {
        return provider_status(provider_error, provider);
    }
    if timed_out(error, route) {
        return call_error(error, provider, Some(408), "");
    }
    call_error(error, provider, None, "ValueError")
}

// a timeout is caught on POST in both forms and on DELETE only in the blocking handler, so elsewhere it reaches the catchall
fn timed_out(error: &ContainerFileOpError, route: Option<CallRoute<'_>>) -> bool {
    let Some(route) = route else {
        return false;
    };
    let caught = route.method == "POST" || (route.method == "DELETE" && !route.asynchronous);
    if !caught {
        return false;
    }
    matches!(
        error,
        ContainerFileOpError::Handler(ContainerFileHttpError::Transfer(TransferError::Transport(
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
        )))
    )
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

fn call_error(
    error: &ContainerFileOpError,
    provider: &str,
    status_code: Option<u16>,
    exception_type: &str,
) -> RawException {
    let message = error.to_string();
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: exception_type.to_owned(),
    }
}
