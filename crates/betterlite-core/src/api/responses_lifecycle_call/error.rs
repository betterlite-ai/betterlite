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
    Read(#[from] ReadError),
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
    pub fn new(error: LifecycleError, provider: &str, redactor: &Redactor) -> Self {
        let raw = raw_exception(&error, provider);
        let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
        let metadata = class.metadata();
        Self {
            class: class_name(&class),
            status: metadata.status_code,
            kind: metadata.error_type,
            param: metadata.param,
            code: metadata
                .openai_code
                .map_or(WireField::Null, WireField::Text),
            message: class_message(&class).to_owned(),
            error,
        }
    }
}

// exception_type()'s original_exception: the read handler's BaseLLMException, or the locally raised ValueError
fn raw_exception(error: &LifecycleError, provider: &str) -> RawException {
    if let LifecycleError::Read(ReadError::Provider(provider_error)) = error {
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

// the exception classes exception_type() raises for the families the read path reaches
const fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::Api(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => "APIError",
    }
}

// the message each raised class carries, which is what a caller reads off the exception
fn class_message(class: &ErrorClass) -> &str {
    match class {
        ErrorClass::Authentication(details) => &details.message,
        ErrorClass::NotFound(details) => &details.message,
        ErrorClass::PermissionDenied(details) => &details.message,
        ErrorClass::BadRequest(details) => &details.message,
        ErrorClass::UnprocessableEntity(details) => &details.message,
        ErrorClass::ContextWindowExceeded(details) => &details.message,
        ErrorClass::ContentPolicyViolation(details) => &details.message,
        ErrorClass::ServiceUnavailable(context)
        | ErrorClass::BadGateway(context)
        | ErrorClass::InternalServer(context)
        | ErrorClass::ApiConnection(context)
        | ErrorClass::ApiResponseValidation(context)
        | ErrorClass::UnsupportedParams(context)
        | ErrorClass::Unclassified(context) => &context.message,
        ErrorClass::Timeout(details) => &details.context.message,
        ErrorClass::RateLimit(details) => &details.context.message,
        ErrorClass::MidStreamFallback(details) => &details.context.message,
        ErrorClass::Api(details) => &details.context.message,
        ErrorClass::BudgetExceeded(details) => details.message.as_deref().unwrap_or_default(),
        ErrorClass::GuardrailRaised(details) => &details.message,
        ErrorClass::JsonSchemaValidation(details) => &details.raw_response,
    }
}
