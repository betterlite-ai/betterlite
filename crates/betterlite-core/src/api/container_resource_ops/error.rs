use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::container_resource_http::ContainerResourceHttpError;
use crate::containers::container_request_parameters::ContainerFieldTypeError;

// exception_type() maps every failure with model "" and the provider the call resolved to
const MODEL: &str = "";

// LiteLLM's mock parse, then ContainerObject(**) or DeleteContainerResult(**), whose failures are ValueErrors too
#[derive(Debug, Error)]
pub enum ContainerOpError {
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
    Decode(ContainerFieldTypeError),
    // the Json-to-Value bridge the params map needs; a failure here is local, not a provider answer
    #[error(transparent)]
    Params(#[from] serde_json::Error),
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    #[error("Container provider config not found for provider: {provider}")]
    ProviderConfigMissing { provider: String },
    #[error("the default request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(LoggingStateError),
    #[error(transparent)]
    Handler(ContainerResourceHttpError),
    #[error(transparent)]
    Encode(ContainerFieldTypeError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct ContainerOpFailure {
    #[source]
    pub error: ContainerOpError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...), run once
#[must_use]
pub fn failure(error: ContainerOpError, provider: &str, redactor: &Redactor) -> ContainerOpFailure {
    let class = match &error {
        ContainerOpError::Decode(field_error) | ContainerOpError::Encode(field_error) => {
            named_field(error.to_string(), field_error.field, provider)
        }
        ContainerOpError::Handler(handler_error) => {
            handler_class(handler_error, provider, redactor)
        }
        // the provider name is the caller's input and the call can never succeed, so LiteLLM's ValueError becomes a 400
        ContainerOpError::UnknownProvider { .. }
        | ContainerOpError::ProviderConfigMissing { .. } => ErrorClass::BadRequest(Box::new(
            ResponseDetails::for_provider(error.to_string(), Some(MODEL.to_owned()), provider),
        )),
        ContainerOpError::MockJson { .. }
        | ContainerOpError::MockModel { .. }
        | ContainerOpError::Params(_)
        | ContainerOpError::Timeout { .. }
        | ContainerOpError::Logging(_) => value_error(error.to_string(), provider, redactor),
    };
    ContainerOpFailure { error, class }
}

fn handler_class(
    error: &ContainerResourceHttpError,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    let (exception, synthesized) = match error {
        ContainerResourceHttpError::Provider {
            exception,
            status_code_is_synthesized,
        } => (exception, *status_code_is_synthesized),
        ContainerResourceHttpError::Call(ContainerFileHttpError::Provider(exception)) => {
            (exception, false)
        }
        ContainerResourceHttpError::Call(ContainerFileHttpError::Config(
            ContainerError::Upstream(exception),
        )) => (exception, false),
        ContainerResourceHttpError::Call(ContainerFileHttpError::ParamType { field, .. }) => {
            return named_field(error.to_string(), field, provider);
        }
        ContainerResourceHttpError::Call(_) | ContainerResourceHttpError::QueryValue { .. } => {
            return value_error(error.to_string(), provider, redactor);
        }
    };
    provider_class(exception, provider, synthesized, redactor)
}

// a provider answer keeps its status, while _handle_error's synthesized one lets the message decide
fn provider_class(
    error: &BaseLlmException,
    provider: &str,
    synthesized: bool,
    redactor: &Redactor,
) -> ErrorClass {
    let mut raw = exception(error.message.clone(), provider, Some(error.status_code));
    raw.status_code_is_synthesized = synthesized;
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

fn value_error(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(message, provider, None);
    "ValueError".clone_into(&mut raw.exception_type);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// docs/compatibility.md rejects a value outside its declared type with a 400 that names the field
fn named_field(message: String, field: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: Some(MODEL.to_owned()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

fn exception(message: String, provider: &str, status_code: Option<u16>) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
