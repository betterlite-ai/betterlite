use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::evals::error::EvalsError;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use super::args::{CreateRunArgs, ListRunsArgs, local_vars_create, local_vars_list};
use crate::api::eval_http::error::EvalHttpError;

// exception_type() sees model=None for every eval run call
const MODEL: &str = "";

#[derive(Debug, Error)]
pub enum EvalRunCallError {
    // litellm.LlmProviders(custom_llm_provider) refused the name
    #[error("'{provider}' is not a valid LlmProviders")]
    UnknownProvider {
        provider: String,
        #[source]
        source: strum::ParseError,
    },
    // the ValueError ProviderConfigManager.get_provider_evals_api_config's None becomes
    #[error("CREATE run is not supported for {provider}")]
    UnsupportedCreate { provider: String },
    #[error("LIST runs is not supported for {provider}")]
    UnsupportedList { provider: String },
    // docs/compatibility.md: a value the declared type cannot hold is a 400 that names the field
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    // extra_body updates CreateRunRequest's own keys in LiteLLM, so any other non-null key has no field
    #[error("extra_body holds '{key}', which is not a field of CreateRunRequest")]
    UnknownBodyKey { key: String },
    #[error(transparent)]
    Config(#[from] EvalsError),
    #[error("the eval run request could not be converted: {source}")]
    Params {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // the sync entry polled a handler that had not answered yet
    #[error("the blocking run call waited on a handler that never answered")]
    Suspended,
    #[error(transparent)]
    Transport(#[from] EvalHttpError),
}

// what LiteLLM's exception_type(...) answers for one call, with the local_vars it was given
#[derive(Debug, Error)]
#[error("{error}")]
pub struct EvalRunFailure {
    #[source]
    pub error: EvalRunCallError,
    pub class: ErrorClass,
    pub local_vars: IndexMap<String, Json>,
}

// LiteLLM's except branch around create_run: exception_type(model=None, custom_llm_provider=..., completion_kwargs=local_vars)
#[must_use]
pub fn create_failure(
    error: EvalRunCallError,
    provider: &str,
    request: &CreateRunArgs<'_>,
    redactor: &Redactor,
) -> EvalRunFailure {
    let class = class_of(&error, provider, redactor);
    EvalRunFailure {
        error,
        class,
        local_vars: local_vars_create(request),
    }
}

// LiteLLM's except branch around list_runs, with its own local_vars
#[must_use]
pub fn list_failure(
    error: EvalRunCallError,
    provider: &str,
    request: &ListRunsArgs<'_>,
    redactor: &Redactor,
) -> EvalRunFailure {
    let class = class_of(&error, provider, redactor);
    EvalRunFailure {
        error,
        class,
        local_vars: local_vars_list(request),
    }
}

// exception_type's branches: a caller mistake, the provider's own answer, or a failure no branch maps
fn class_of(error: &EvalRunCallError, provider: &str, redactor: &Redactor) -> ErrorClass {
    let message = error.to_string();
    match error {
        EvalRunCallError::UnknownProvider { .. }
        | EvalRunCallError::UnsupportedCreate { .. }
        | EvalRunCallError::UnsupportedList { .. } => provider_class(message, provider),
        EvalRunCallError::FieldType { field, .. } => field_class(field, message, provider),
        EvalRunCallError::UnknownBodyKey { key } => field_class(key, message, provider),
        EvalRunCallError::Config(source) => evals_class(source, provider, redactor),
        EvalRunCallError::Transport(source) => http_class(source, provider, redactor),
        EvalRunCallError::Params { .. }
        | EvalRunCallError::Logging(_)
        | EvalRunCallError::Suspended => local_class(message, provider, redactor),
    }
}

// the provider config's own error: a provider answer maps through B185, the rest is the config's own ValueError
fn evals_class(error: &EvalsError, provider: &str, redactor: &Redactor) -> ErrorClass {
    match error {
        EvalsError::Provider { provider: found } => provider_exception(found, provider, redactor),
        EvalsError::ApiKeyRequired
        | EvalsError::PathSegment { .. }
        | EvalsError::ReplyTooLarge { .. }
        | EvalsError::ReplyJson { .. }
        | EvalsError::ReplyModel { .. }
        | EvalsError::RequestEncode { .. }
        | EvalsError::RequestNotObject => local_class(error.to_string(), provider, redactor),
    }
}

// the B9371 handler's failure: its provider answer here, its logging and recording failures below
fn http_class(error: &EvalHttpError, provider: &str, redactor: &Redactor) -> ErrorClass {
    match error {
        EvalHttpError::Api(source) => evals_class(source, provider, redactor),
        EvalHttpError::Logging(_) | EvalHttpError::Record { .. } => {
            local_class(error.to_string(), provider, redactor)
        }
    }
}

// the provider exception exception_type maps, carrying the status its reply gave
fn provider_exception(found: &ProviderError, provider: &str, redactor: &Redactor) -> ErrorClass {
    let raw = RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: found.message.clone(),
        status_code: Some(found.status_code),
        status_code_is_synthesized: found.status_code_is_synthesized,
        message: Some(found.message.clone()),
        body: None,
        exception_type: String::new(),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// an exception outside the provider's family: no branch matches, and with no status it reports api_connection
fn local_class(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    let raw = RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// the provider name is the caller's input and the call can never succeed, so LiteLLM's ValueError becomes a 400
fn provider_class(message: String, provider: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::for_provider(
        message,
        Some(MODEL.to_owned()),
        provider,
    )))
}

// docs/compatibility.md: a value the declared type cannot hold is a 400 with the field as its param
fn field_class(field: &str, message: String, provider: &str) -> ErrorClass {
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
