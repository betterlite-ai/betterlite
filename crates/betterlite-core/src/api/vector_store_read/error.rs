use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::service::Operation;
use crate::api::assistant_records::error::class_name;
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_write::error::{BaseLlmException, WriteError};
use crate::api::vector_store_search::error::VectorStoreSearchError;
use crate::api::vector_store_write::error::RaisedException;

// exception_type() maps every read failure with model None and the provider the call resolved to
const MODEL: &str = "";

#[derive(Debug, Error)]
pub enum VectorStoreReadError {
    // B174's resolution of the provider name and the api_type a slash carried
    #[error(transparent)]
    Resolution(#[from] VectorStoreSearchError),
    // the resolved provider is one no config source serves
    #[error("Vector store {} is not supported for {provider}", operation.label())]
    Unconfigured {
        operation: Operation,
        provider: String,
    },
    #[error("the request timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // B2485's HTTP read, whose provider errors carry the status exception_type() maps by
    #[error(transparent)]
    Read(#[from] CreateReadError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct VectorStoreReadFailure {
    #[source]
    pub error: VectorStoreReadError,
    pub class: ErrorClass,
}

impl VectorStoreReadFailure {
    // the exception a Python caller catches, as exception_type() raises it
    pub fn raised(&self) -> RaisedException {
        let metadata = self.class.metadata();
        RaisedException {
            class: class_name(&self.class),
            message: class_message(&self.class),
            status: metadata.status_code,
            kind: metadata.error_type,
            param: metadata.param,
            code: metadata.openai_code,
        }
    }
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(
    error: VectorStoreReadError,
    provider: &str,
    redactor: &Redactor,
) -> VectorStoreReadFailure {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(&error, provider))),
        redactor,
    );
    VectorStoreReadFailure { error, class }
}

fn raw_exception(error: &VectorStoreReadError, provider: &str) -> RawException {
    if let VectorStoreReadError::Read(source) = error
        && let Some(exception) = provider_exception(source)
    {
        return RawException {
            model: MODEL.to_owned(),
            custom_llm_provider: provider.to_owned(),
            error_str: exception.message.clone(),
            status_code: Some(exception.status_code),
            status_code_is_synthesized: exception.status_code_is_synthesized,
            message: Some(exception.message.clone()),
            body: None,
            exception_type: String::new(),
        };
    }
    let message = error.to_string();
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::from("ValueError"),
    }
}

fn provider_exception(error: &CreateReadError) -> Option<&BaseLlmException> {
    if let CreateReadError::Write(WriteError::Provider(exception)) = error {
        return Some(exception);
    }
    None
}
