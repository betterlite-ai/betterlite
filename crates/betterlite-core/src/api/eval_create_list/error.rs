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

use super::DEFAULT_EVALS_PROVIDER;
use super::call::EvalsCallParams;
use crate::api::eval_http::error::{EvalHttpError, unsupported_provider};

// what create_eval and list_evals raise before litellm.exception_type maps it
#[derive(Debug, Error)]
pub enum EvalsCreateListError {
    // LlmProviders(custom_llm_provider) rejects a name outside the provider set
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    #[error("CREATE eval is not supported for {provider}")]
    CreateUnsupported { provider: String },
    #[error("LIST evals is not supported for {provider}")]
    ListUnsupported { provider: String },
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

impl From<EvalsError> for EvalsCreateListError {
    fn from(source: EvalsError) -> Self {
        Self::Handler(EvalHttpError::from(source))
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct EvalsCreateListFailureError {
    #[source]
    pub error: EvalsCreateListError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=..., completion_kwargs=local_vars)
#[must_use]
pub fn failure(
    error: EvalsCreateListError,
    params: &EvalsCallParams<'_>,
    redactor: &Redactor,
) -> EvalsCreateListFailureError {
    let provider = params.custom_llm_provider.unwrap_or(DEFAULT_EVALS_PROVIDER);
    let class = match &error {
        EvalsCreateListError::FieldType { name } => {
            named_field_class(error.to_string(), name, provider)
        }
        EvalsCreateListError::Handler(EvalHttpError::Api(EvalsError::Provider {
            provider: provider_error,
        })) => provider_class(provider_error, provider, redactor),
        EvalsCreateListError::UnknownProvider { .. }
        | EvalsCreateListError::CreateUnsupported { .. }
        | EvalsCreateListError::ListUnsupported { .. } => {
            unsupported_provider(error.to_string(), provider)
        }
        // everything else outside the provider family reaches exception_type as a ValueError
        EvalsCreateListError::Timeout { .. }
        | EvalsCreateListError::Params(_)
        | EvalsCreateListError::Logging(_)
        | EvalsCreateListError::Handler(_) => {
            let raw_value = RawFailure {
                provider,
                message: error.to_string(),
                status_code: None,
                status_code_is_synthesized: false,
                body: None,
                exception_type: "ValueError",
            };
            raw_value.class(redactor)
        }
    };
    EvalsCreateListFailureError { error, class }
}

// the raw exception both failure paths hand to exception_type's dispatch
struct RawFailure<'call> {
    provider: &'call str,
    message: String,
    status_code: Option<u16>,
    status_code_is_synthesized: bool,
    body: Option<RawExceptionBody>,
    exception_type: &'call str,
}

impl RawFailure<'_> {
    fn class(self, redactor: &Redactor) -> ErrorClass {
        let raw = RawException {
            model: String::new(),
            custom_llm_provider: self.provider.to_owned(),
            error_str: self.message.clone(),
            status_code: self.status_code,
            status_code_is_synthesized: self.status_code_is_synthesized,
            message: Some(self.message),
            body: self.body,
            exception_type: self.exception_type.to_owned(),
        };
        dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
    }
}

// a provider answer keeps the status and class exception_type gives it
fn provider_class(error: &ProviderError, provider: &str, redactor: &Redactor) -> ErrorClass {
    RawFailure {
        provider,
        message: error.message.clone(),
        status_code: Some(error.status_code),
        status_code_is_synthesized: error.status_code_is_synthesized,
        body: error.body.as_ref().map(provider_body),
        exception_type: "",
    }
    .class(redactor)
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
