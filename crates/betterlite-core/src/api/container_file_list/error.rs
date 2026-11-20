use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::containers::container_request_parameters::ContainerFieldTypeError;

// exception_type() maps every file-list failure with model "" and the provider the call resolved to
const MODEL: &str = "";

// LiteLLM's mock parse, then ContainerFileListResponse(**mock_response), whose failures are ValueErrors too
#[derive(Debug, Error)]
pub enum MockDecodeError {
    #[error("the mock response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match a container file list")]
    Model {
        #[source]
        source: serde_json::Error,
    },
}

// what the file-list handler raises before exception_type sees it
#[derive(Debug, Error)]
pub enum FileListHandlerError {
    // read_params refuses a credential or ssl_verify of the wrong type
    #[error(transparent)]
    Params(#[from] ContainerFileHttpError),
    // get_complete_url and the two provider transforms
    #[error(transparent)]
    Config(#[from] ContainerError),
    #[error(transparent)]
    Client(#[from] PoolError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    // the params a provider transform answers are bridged to typed query values here
    #[error(transparent)]
    Encoding(#[from] serde_json::Error),
    #[error("the {field} query parameter does not have a single value")]
    QueryValue { field: String },
    // _handle_error's answer: the config's own exception for a transport failure, at the status betterlite synthesized
    #[error(transparent)]
    Transport(BaseLlmException),
    // raise_for_error_status' answer, which keeps the status the provider sent
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
}

#[derive(Debug, Error)]
pub enum ContainerFileListError {
    #[error(transparent)]
    Mock(#[from] MockDecodeError),
    #[error(transparent)]
    Decode(#[from] ContainerFieldTypeError),
    // the Json-to-JsonValue and JsonValue-to-Object bridges in prepare.rs; a failure here is local, not a provider answer
    #[error(transparent)]
    Params(#[from] serde_json::Error),
    #[error("Container provider config not found for provider: {provider}")]
    ProviderConfigMissing { provider: String },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] FileListHandlerError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct ContainerFileListFailure {
    #[source]
    pub error: ContainerFileListError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(
    error: ContainerFileListError,
    provider: &str,
    redactor: &Redactor,
) -> ContainerFileListFailure {
    let class = match &error {
        ContainerFileListError::Decode(field_error) => {
            named_field_class(error.to_string(), field_error.field, provider)
        }
        ContainerFileListError::Handler(handler_error) => {
            handler_class(handler_error, provider, redactor)
        }
        ContainerFileListError::Mock(_)
        | ContainerFileListError::Params(_)
        | ContainerFileListError::ProviderConfigMissing { .. }
        | ContainerFileListError::Logging(_) => value_error(error.to_string(), provider, redactor),
    };
    ContainerFileListFailure { error, class }
}

fn handler_class(error: &FileListHandlerError, provider: &str, redactor: &Redactor) -> ErrorClass {
    let (exception, synthesized) = match error {
        FileListHandlerError::Params(ContainerFileHttpError::ParamType { field, .. }) => {
            return named_field_class(error.to_string(), *field, provider);
        }
        FileListHandlerError::Provider(provider_error) => (provider_error, false),
        FileListHandlerError::Transport(transport_error) => (transport_error, true),
        FileListHandlerError::Config(ContainerError::Upstream(upstream)) => (upstream, false),
        FileListHandlerError::Params(_)
        | FileListHandlerError::Config(_)
        | FileListHandlerError::Client(_)
        | FileListHandlerError::Logging(_)
        | FileListHandlerError::Encoding(_)
        | FileListHandlerError::QueryValue { .. } => {
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
fn named_field_class(message: String, field: &str, provider: &str) -> ErrorClass {
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
