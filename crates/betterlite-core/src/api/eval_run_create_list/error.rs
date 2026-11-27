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

impl EvalRunFailure {
    // LiteLLM's except branch around create_run or list_runs: exception_type(model=None, custom_llm_provider=..., completion_kwargs=local_vars)
    #[must_use]
    pub fn of(
        error: EvalRunCallError,
        provider: &str,
        redactor: &Redactor,
        local_vars: IndexMap<String, Json>,
    ) -> Self {
        let class = class_of(&error, provider, redactor);
        Self {
            error,
            class,
            local_vars,
        }
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
        EvalRunCallError::Config(EvalsError::Provider { provider: found })
        | EvalRunCallError::Transport(EvalHttpError::Api(EvalsError::Provider {
            provider: found,
        })) => provider_exception(found, provider, redactor),
        EvalRunCallError::Config(_)
        | EvalRunCallError::Transport(_)
        | EvalRunCallError::Params { .. }
        | EvalRunCallError::Logging(_)
        | EvalRunCallError::Suspended => local_class(message, provider, redactor),
    }
}

// the raw exception B185's dispatch maps: the message, the status a reply gave and the provider that answered
fn raw_exception(
    message: String,
    status_code: Option<u16>,
    status_code_is_synthesized: bool,
    provider: &str,
) -> RawException {
    RawException {
        model: MODEL.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized,
        message: Some(message),
        body: None,
        exception_type: String::new(),
    }
}

// the provider exception exception_type maps, carrying the status its reply gave
fn provider_exception(found: &ProviderError, provider: &str, redactor: &Redactor) -> ErrorClass {
    let raw = raw_exception(
        found.message.clone(),
        Some(found.status_code),
        found.status_code_is_synthesized,
        provider,
    );
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// an exception outside the provider's family: no branch matches, and with no status it reports api_connection
fn local_class(message: String, provider: &str, redactor: &Redactor) -> ErrorClass {
    dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(message, None, false, provider))),
        redactor,
    )
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
