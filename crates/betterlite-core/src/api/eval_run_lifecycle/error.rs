use std::io;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::evals::error::EvalsError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::eval_http::error::EvalHttpError;

// The three run operations LiteLLM spells out twice each; one call body serves them, keyed by this
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    Get,
    Cancel,
    Delete,
}

impl RunAction {
    // The word LiteLLM puts in `ValueError(f"{label} run is not supported for {provider}")`
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Cancel => "CANCEL",
            Self::Delete => "DELETE",
        }
    }

    // The @client wrapper's flag, which the sync body pops before GenericLiteLLMParams(**kwargs)
    #[must_use]
    pub const fn async_flag(self) -> &'static str {
        match self {
            Self::Get => "aget_run",
            Self::Cancel => "acancel_run",
            Self::Delete => "adelete_run",
        }
    }
}

// Everything one run call can fail with before, during and after the provider lookup
#[derive(Debug, Error)]
pub enum EvalRunError {
    // litellm.LlmProviders(name) rejects a name outside its set with this ValueError
    #[error("'{provider}' is not a valid LlmProviders")]
    UnknownProvider { provider: String },
    // the None branch of ProviderConfigManager.get_provider_evals_api_config
    #[error("{} run is not supported for {provider}", action.label())]
    Unsupported { action: RunAction, provider: String },
    // GenericLiteLLMParams declares api_base as a string, and pydantic refuses anything else
    #[error("api_base must be a string")]
    ApiBase,
    #[error("the run parameters do not serialize: {source}")]
    Params {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Evals(#[from] EvalsError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] EvalHttpError),
    // B9371 has async handlers only, so a blocking call that is already inside a runtime cannot be driven
    #[error("a blocking eval run call cannot run inside an async runtime")]
    SyncInRuntime,
    #[error("the blocking eval run runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct EvalRunFailure {
    #[source]
    pub error: EvalRunError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...), run once
#[must_use]
pub fn failure(error: EvalRunError, provider: &str, redactor: &Redactor) -> EvalRunFailure {
    let class = match &error {
        EvalRunError::ApiBase => named_field(error.to_string(), "api_base", provider),
        EvalRunError::Evals(source) => evals_class(source, provider, redactor),
        EvalRunError::Handler(EvalHttpError::Api(source)) => {
            evals_class(source, provider, redactor)
        }
        EvalRunError::UnknownProvider { .. }
        | EvalRunError::Unsupported { .. }
        | EvalRunError::Handler(EvalHttpError::Logging(_) | EvalHttpError::Record { .. })
        | EvalRunError::Logging(_)
        | EvalRunError::Params { .. }
        | EvalRunError::SyncInRuntime
        | EvalRunError::Runtime { .. } => value_error(error.to_string(), provider, redactor),
    };
    EvalRunFailure { error, class }
}

// A kwarg the declared type cannot hold is a 400 that names the field, as docs/compatibility.md asks
fn named_field(message: String, field: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: Some(String::new()),
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(field.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}

// a provider answer keeps its status, while every other eval failure reaches exception_type as a ValueError
fn evals_class(error: &EvalsError, provider: &str, redactor: &Redactor) -> ErrorClass {
    if let EvalsError::Provider { provider: cause } = error {
        let mut raw = exception(cause.message.clone(), provider, Some(cause.status_code));
        raw.status_code_is_synthesized = cause.status_code_is_synthesized;
        return dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    }
    value_error(error.to_string(), provider, redactor)
}

fn value_error(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    let mut raw = exception(message, provider, None);
    "ValueError".clone_into(&mut raw.exception_type);
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

fn exception(message: String, provider: &str, status: Option<u16>) -> RawException {
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: status,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}
