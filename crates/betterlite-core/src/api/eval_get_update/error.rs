use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_openai::error::ProviderError;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::eval_create_list::DEFAULT_EVALS_PROVIDER;
use crate::api::eval_create_list::call::EvalsCallParams;
use crate::api::eval_http::error::{EvalHttpError, unsupported_provider};

// what get_eval and update_eval raise before litellm.exception_type maps it
#[derive(Debug, Error)]
pub enum EvalsGetUpdateError {
    // LlmProviders(custom_llm_provider) rejects a name outside the provider set
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    #[error("GET eval is not supported for {provider}")]
    GetUnsupported { provider: String },
    #[error("UPDATE eval is not supported for {provider}")]
    UpdateUnsupported { provider: String },
    // docs/compatibility.md rejects a value outside its declared type, naming the field
    #[error("the {name} field does not have the declared type")]
    FieldType { name: String },
    #[error("the request timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    // the Json-to-Value bridge the provider transforms need; a failure here is local, not a provider answer
    #[error(transparent)]
    Params(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    // a validate_environment, transform or handler failure, which EvalHttpError already merges
    #[error(transparent)]
    Handler(#[from] EvalHttpError),
}

impl From<EvalsError> for EvalsGetUpdateError {
    fn from(source: EvalsError) -> Self {
        Self::Handler(EvalHttpError::from(source))
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct EvalsGetUpdateFailureError {
    #[source]
    pub error: EvalsGetUpdateError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=..., completion_kwargs=local_vars)
#[must_use]
pub fn failure(
    error: EvalsGetUpdateError,
    params: &EvalsCallParams<'_>,
    redactor: &Redactor,
) -> EvalsGetUpdateFailureError {
    let provider = params.custom_llm_provider.unwrap_or(DEFAULT_EVALS_PROVIDER);
    let class = match &error {
        EvalsGetUpdateError::FieldType { name } => {
            named_field_class(error.to_string(), name, provider)
        }
        EvalsGetUpdateError::Handler(EvalHttpError::Api(EvalsError::Provider {
            provider: provider_error,
        })) => dispatch(
            &DispatchInput::Raw(Box::new(raw_exception(
                provider,
                &FailureSource::Provider(provider_error),
            ))),
            redactor,
        ),
        EvalsGetUpdateError::UnknownProvider { .. }
        | EvalsGetUpdateError::GetUnsupported { .. }
        | EvalsGetUpdateError::UpdateUnsupported { .. } => {
            unsupported_provider(error.to_string(), provider)
        }
        // everything else outside the provider family reaches exception_type as a ValueError
        EvalsGetUpdateError::Timeout { .. }
        | EvalsGetUpdateError::Params(_)
        | EvalsGetUpdateError::Logging(_)
        | EvalsGetUpdateError::Handler(_) => dispatch(
            &DispatchInput::Raw(Box::new(raw_exception(
                provider,
                &FailureSource::Local(&error),
            ))),
            redactor,
        ),
    };
    EvalsGetUpdateFailureError { error, class }
}

// which failure asks for a raw exception: a provider answer keeps its status, a local failure has none
enum FailureSource<'error> {
    Provider(&'error ProviderError),
    Local(&'error EvalsGetUpdateError),
}

// what the two failure paths differ in; exception_type picks its arm from the status and the type
struct ExceptionFacts {
    message: String,
    status_code: Option<u16>,
    status_code_is_synthesized: bool,
    body: Option<RawExceptionBody>,
    exception_type: &'static str,
}

// the raw exception exception_type's dispatch classifies
fn raw_exception(provider: &str, source: &FailureSource<'_>) -> RawException {
    let facts = match source {
        FailureSource::Provider(error) => ExceptionFacts {
            message: error.message.clone(),
            status_code: Some(error.status_code),
            status_code_is_synthesized: error.status_code_is_synthesized,
            body: error.body.as_ref().map(provider_body),
            exception_type: "",
        },
        FailureSource::Local(error) => ExceptionFacts {
            message: error.to_string(),
            status_code: None,
            status_code_is_synthesized: false,
            body: None,
            exception_type: "ValueError",
        },
    };
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: facts.message.clone(),
        status_code: facts.status_code,
        status_code_is_synthesized: facts.status_code_is_synthesized,
        message: Some(facts.message),
        body: facts.body,
        exception_type: facts.exception_type.to_owned(),
    }
}

fn provider_body(body: &Object) -> RawExceptionBody {
    RawExceptionBody {
        kind: body_text(body, "type"),
        param: body_text(body, "param"),
        code: body_text(body, "code"),
        message: body_text(body, "message"),
        error: None,
    }
}

fn body_text(body: &Object, name: &str) -> Option<String> {
    body.get(name)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

// docs/compatibility.md rejects a value outside its declared type with a 400 that names the field
fn named_field_class(message: String, name: &str, provider: &str) -> ErrorClass {
    let context = ErrorContext {
        message,
        model: None,
        provider: Some(provider.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(name.to_owned()),
        body_code: None,
    };
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, None)))
}
