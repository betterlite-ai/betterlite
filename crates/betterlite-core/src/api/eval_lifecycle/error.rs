use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::error::ProviderError;
use betterlite_openai::evals::error::EvalsError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::eval_http::error::EvalHttpError;

// exception_type() maps every eval lifecycle failure with model ""
const MODEL: &str = "";

#[derive(Debug, Error)]
pub enum EvalLifecycleError {
    #[error("{operation} eval is not supported for {provider}")]
    Unsupported {
        operation: &'static str,
        provider: String,
    },
    #[error(transparent)]
    Config(#[from] EvalsError),
    #[error("the call keywords do not encode as request parameters")]
    Params {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] EvalHttpError),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct EvalLifecycleFailure {
    #[source]
    pub error: EvalLifecycleError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(
    error: EvalLifecycleError,
    provider: &str,
    redactor: &Redactor,
) -> EvalLifecycleFailure {
    let class = match &error {
        EvalLifecycleError::Unsupported { .. } => unsupported(&error, provider),
        EvalLifecycleError::Config(EvalsError::Provider { provider: cause })
        | EvalLifecycleError::Handler(EvalHttpError::Api(EvalsError::Provider {
            provider: cause,
        })) => provider_class(cause, provider, redactor),
        EvalLifecycleError::Config(_)
        | EvalLifecycleError::Params { .. }
        | EvalLifecycleError::Logging(_)
        | EvalLifecycleError::Handler(_) => {
            plain(&error.to_string(), provider, "ValueError", redactor)
        }
    };
    EvalLifecycleFailure { error, class }
}

// the provider name is the caller's input and the call can never succeed, so LiteLLM's ValueError becomes a 400
fn unsupported(error: &EvalLifecycleError, provider: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::for_provider(
        error.to_string(),
        Some(MODEL.to_owned()),
        provider,
    )))
}

// get_error_class's BaseLLMException is raised by the handler, so exception_type sees its message and status
fn provider_class(error: &ProviderError, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(
        &error.message,
        provider,
        Some(error.status_code),
        "BaseLLMException",
    );
    raw.status_code_is_synthesized = error.status_code_is_synthesized;
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

fn plain(message: &str, provider: &str, exception_type: &str, redactor: &Redactor) -> ErrorClass {
    dispatch(
        &DispatchInput::Raw(Box::new(exception(message, provider, None, exception_type))),
        redactor,
    )
}

fn exception(
    message: &str,
    provider: &str,
    status_code: Option<u16>,
    exception_type: &str,
) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body: None,
        exception_type: exception_type.to_owned(),
    }
}
