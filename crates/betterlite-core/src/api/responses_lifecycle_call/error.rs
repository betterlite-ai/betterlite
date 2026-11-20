use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::response_id_traversal::response_id::ResponseRewriteError;
use crate::api::responses_read_http::error::ReadError;

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("custom_llm_provider is required but passed as None")]
    MissingProvider,
    #[error("GET responses is not supported for {provider}")]
    UnsupportedRetrieve { provider: String },
    #[error("list_input_items is not supported for {provider}")]
    UnsupportedInputItems { provider: String },
    #[error(transparent)]
    Read(Box<ReadError>),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Rewrite(#[from] ResponseRewriteError),
    #[error("the read answered input items where a response was expected")]
    UnexpectedInputItems,
    #[error("the read answered a response where input items were expected")]
    UnexpectedResponse,
}

// the SDK exception a caller observes: the class the pinned SDK raises and the flat fields its APIError carries
#[derive(Debug)]
pub struct LifecycleFailure {
    pub error: LifecycleError,
    pub class: &'static str,
    pub status: u16,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

impl LifecycleFailure {
    #[must_use]
    pub fn new(error: LifecycleError, provider: Option<&str>, redactor: &Redactor) -> Self {
        let raw = raw_exception(&error, provider.unwrap_or_default());
        let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
        let metadata = class.metadata();
        let (name, text) = class_and_message(&class);
        Self {
            class: name,
            status: metadata.status_code,
            kind: metadata.error_type,
            param: metadata.param,
            code: metadata
                .openai_code
                .map_or(WireField::Null, WireField::Text),
            message: text.to_owned(),
            error,
        }
    }
}

// exception_type()'s original_exception: the read handler's BaseLLMException, or the locally raised ValueError
fn raw_exception(error: &LifecycleError, provider: &str) -> RawException {
    if let LifecycleError::Read(read) = error
        && let ReadError::Provider(provider_error) = &**read
    {
        return RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: provider_error.message.clone(),
            status_code: Some(provider_error.status_code),
            status_code_is_synthesized: provider_error.status_code_is_synthesized,
            message: Some(provider_error.message.clone()),
            body: None,
            exception_type: String::from("BaseLLMException"),
        };
    }
    RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: error.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    }
}

// the exception class exception_type() raises for the families the read path reaches, and the message it carries
fn class_and_message(class: &ErrorClass) -> (&'static str, &str) {
    match class {
        ErrorClass::Authentication(details) => ("AuthenticationError", &details.message),
        ErrorClass::NotFound(details) => ("NotFoundError", &details.message),
        ErrorClass::PermissionDenied(details) => ("PermissionDeniedError", &details.message),
        ErrorClass::BadRequest(details) => ("BadRequestError", &details.message),
        ErrorClass::UnprocessableEntity(details) => ("UnprocessableEntityError", &details.message),
        ErrorClass::ContextWindowExceeded(details) => {
            ("ContextWindowExceededError", &details.message)
        }
        ErrorClass::ContentPolicyViolation(details) => {
            ("ContentPolicyViolationError", &details.message)
        }
        ErrorClass::ServiceUnavailable(context) => ("ServiceUnavailableError", &context.message),
        ErrorClass::BadGateway(context) => ("BadGatewayError", &context.message),
        ErrorClass::InternalServer(context) => ("InternalServerError", &context.message),
        ErrorClass::Timeout(details) => ("Timeout", &details.context.message),
        ErrorClass::RateLimit(details) => ("RateLimitError", &details.context.message),
        ErrorClass::ApiConnection(context) => ("APIConnectionError", &context.message),
        ErrorClass::ApiResponseValidation(context) => {
            ("APIResponseValidationError", &context.message)
        }
        ErrorClass::BudgetExceeded(details) => (
            "BudgetExceededError",
            details.message.as_deref().unwrap_or_default(),
        ),
        ErrorClass::UnsupportedParams(context) => ("UnsupportedParamsError", &context.message),
        ErrorClass::GuardrailRaised(details) => ("APIError", &details.message),
        ErrorClass::JsonSchemaValidation(details) => ("APIError", &details.raw_response),
        ErrorClass::MidStreamFallback(details) => ("APIError", &details.context.message),
        ErrorClass::Api(details) => ("APIError", &details.context.message),
        ErrorClass::Unclassified(context) => ("APIError", &context.message),
    }
}
