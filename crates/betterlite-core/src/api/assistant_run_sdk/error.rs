use std::error::Error;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::TimeoutError;

#[derive(Debug, Error)]
pub enum AssistantRunError {
    // run_thread builds this itself, so no mapping ever sees it
    #[error("LiteLLM doesn't support {provider} for 'run_thread'. Only 'openai' is supported.")]
    Unsupported { provider: String },
    // the interpreter raises this before run_thread runs, so the text is its own
    #[error(
        "litellm.assistants.main.run_thread() got multiple values for keyword argument 'stream'"
    )]
    DuplicateStream,
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("could not convert timeout {seconds} to a duration")]
    InvalidTimeout { seconds: f64 },
    #[error("could not convert string to float: '{text}'")]
    InvalidTimeoutText { text: String },
    #[error("extra_body must be an object or null")]
    InvalidExtraBody,
    #[error(transparent)]
    Dispatch(#[from] AssistantsRunError),
    #[error(transparent)]
    Mapped(#[from] RunFailure),
}

impl AssistantRunError {
    // the response litellm builds for its own BadRequestError, which the client can read back
    #[must_use]
    pub const fn response(&self) -> Option<(&'static str, &'static str)> {
        if matches!(self, Self::Unsupported { .. }) {
            Some(("Unsupported provider", "CREATE_THREAD"))
        } else {
            None
        }
    }
}

// the exception_type mapping's result: the litellm exception class and the message it carries
#[derive(Debug, Error)]
#[error("{message}")]
pub struct RunFailure {
    pub class: ErrorClass,
    pub class_name: &'static str,
    pub message: String,
}

// what the mapping needs from a provider failure: the SDK status and body, or the text it raises
pub trait RunDispatchError: Error + Send + Sync + 'static {
    fn status_and_body(&self) -> Option<(u16, &[u8])>;
    fn message(&self) -> String;
}

// LiteLLM's exception_type(model="", custom_llm_provider=..., original_exception=..., completion_kwargs={})
#[must_use]
pub fn map_failure(
    provider: &str,
    error: &(impl RunDispatchError + ?Sized),
    redactor: &Redactor,
) -> RunFailure {
    let text = error.message();
    let response = error.status_and_body();
    let status = response.as_ref().map(|(status, _)| *status);
    let body = response.and_then(|(_, bytes)| sdk_body(bytes));
    failed(provider, text, status, body, redactor)
}

// LiteLLM's exception classes prefix their own name to the message exception_type produced
#[must_use]
pub fn failed(
    provider: &str,
    text: String,
    status: Option<u16>,
    body: Option<RawExceptionBody>,
    redactor: &Redactor,
) -> RunFailure {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: text.clone(),
            status_code: status,
            status_code_is_synthesized: false,
            message: Some(text),
            body,
            exception_type: String::new(),
        })),
        redactor,
    );
    let (class_name, inner) = describe(&class);
    RunFailure {
        message: format!("litellm.{class_name}: {inner}"),
        class,
        class_name,
    }
}

// the openai SDK reads the error object, or the whole body when it carries no "error" key
fn sdk_body(bytes: &[u8]) -> Option<RawExceptionBody> {
    let data = serde_json::from_slice::<JsonValue>(bytes).ok()?;
    let error = data.get("error").unwrap_or(&data).as_object()?;
    let text = |name: &str| {
        error
            .get(name)
            .and_then(JsonValue::as_str)
            .map(str::to_owned)
    };
    Some(RawExceptionBody {
        kind: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message"),
        error: None,
    })
}

// the class each mapping raises, by the name Python reports for it, and the message it carries
fn describe(class: &ErrorClass) -> (&'static str, &str) {
    match class {
        ErrorClass::Authentication(found) => ("AuthenticationError", found.message.as_str()),
        ErrorClass::NotFound(found) => ("NotFoundError", found.message.as_str()),
        ErrorClass::PermissionDenied(found) => ("PermissionDeniedError", found.message.as_str()),
        ErrorClass::BadRequest(found) => ("BadRequestError", found.message.as_str()),
        ErrorClass::UnprocessableEntity(found) => {
            ("UnprocessableEntityError", found.message.as_str())
        }
        ErrorClass::ContextWindowExceeded(found) => {
            ("ContextWindowExceededError", found.message.as_str())
        }
        ErrorClass::ContentPolicyViolation(found) => {
            ("ContentPolicyViolationError", found.message.as_str())
        }
        ErrorClass::ServiceUnavailable(found) => {
            ("ServiceUnavailableError", found.message.as_str())
        }
        ErrorClass::BadGateway(found) => ("BadGatewayError", found.message.as_str()),
        ErrorClass::InternalServer(found) => ("InternalServerError", found.message.as_str()),
        ErrorClass::Timeout(found) => ("Timeout", found.context.message.as_str()),
        ErrorClass::RateLimit(found) => ("RateLimitError", found.context.message.as_str()),
        ErrorClass::Api(found) => ("APIError", found.context.message.as_str()),
        ErrorClass::ApiConnection(found) => ("APIConnectionError", found.message.as_str()),
        ErrorClass::ApiResponseValidation(found) => {
            ("APIResponseValidationError", found.message.as_str())
        }
        ErrorClass::JsonSchemaValidation(_) => ("JSONSchemaValidationError", ""),
        ErrorClass::UnsupportedParams(found) => ("UnsupportedParamsError", found.message.as_str()),
        ErrorClass::BudgetExceeded(found) => (
            "BudgetExceededError",
            found.message.as_deref().unwrap_or_default(),
        ),
        ErrorClass::GuardrailRaised(found) => ("GuardrailRaisedError", found.message.as_str()),
        ErrorClass::MidStreamFallback(found) => {
            ("MidStreamFallbackError", found.context.message.as_str())
        }
        ErrorClass::Unclassified(found) => ("Exception", found.message.as_str()),
    }
}

// both provider branches report runs through this enum, so one mapping covers them
impl RunDispatchError for AssistantsRunError {
    fn status_and_body(&self) -> Option<(u16, &[u8])> {
        match self {
            Self::Runs(ThreadRunsError::Status(status, body)) => Some((*status, body.as_slice())),
            Self::Client(_) | Self::Runs(_) | Self::Encode(_) => None,
        }
    }

    // str(openai.APIStatusError): "Error code: {status} - {body}"
    fn message(&self) -> String {
        match self.status_and_body() {
            Some((status, body)) => format!(
                "Error code: {status} - {}",
                str::from_utf8(body).unwrap_or_default()
            ),
            None => self.to_string(),
        }
    }
}
