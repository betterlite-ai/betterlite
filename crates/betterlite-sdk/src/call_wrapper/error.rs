use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use thiserror::Error;

use super::messages::error_message;
use crate::call_setup::credentials::CredentialError;
use crate::call_setup::function_setup::CallSetupError;
use crate::call_setup::hooks::HookError;
use crate::call_setup::text::DebugTextError;

// RawRequestCaptured is LiteLLM's sentinel: the provider translation stopped before sending to record the request
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CallErrorKind {
    #[default]
    Normal,
    RawRequestCaptured,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}", error_message(.class))]
pub struct CallError {
    pub class: ErrorClass,
    pub kind: CallErrorKind,
    // LiteLLM stamps these on the exception in the async wrapper only
    pub num_retries: Option<i64>,
    pub timeout: Option<Json>,
}

impl From<ErrorClass> for CallError {
    fn from(class: ErrorClass) -> Self {
        Self {
            class,
            kind: CallErrorKind::Normal,
            num_retries: None,
            timeout: None,
        }
    }
}

impl From<CallSetupError> for CallError {
    fn from(error: CallSetupError) -> Self {
        match error {
            CallSetupError::Rule(rule) => Self::from(rule.classify()),
            other @ CallSetupError::Logging(_)
            | other @ CallSetupError::Guardrail(_)
            | other @ CallSetupError::Callback(_)
            | other @ CallSetupError::FieldType { .. }
            | other @ CallSetupError::MissingField { .. }
            | other @ CallSetupError::Json { .. } => Self::from(plain_error(&other.to_string())),
        }
    }
}

impl From<CredentialError> for CallError {
    fn from(error: CredentialError) -> Self {
        Self::from(plain_error(&error.to_string()))
    }
}

impl From<HookError> for CallError {
    fn from(error: HookError) -> Self {
        Self::from(plain_error(&error.to_string()))
    }
}

impl From<DebugTextError> for CallError {
    fn from(error: DebugTextError) -> Self {
        Self::from(plain_error(&error.to_string()))
    }
}

// LiteLLM raises a bare Exception for these, so the class carries nothing but the message
#[must_use]
pub fn plain_error(message: &str) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

#[must_use]
pub fn field_error(field: &str, expected: &str) -> CallError {
    CallError::from(plain_error(&format!(
        "the {field} value must be {expected}"
    )))
}

// The class name Python's type(e).__name__ reports, so a wire report can compare the two sides
#[must_use]
pub const fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Api(_) => "APIError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::JsonSchemaValidation(_) => "JSONSchemaValidationError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedException",
        ErrorClass::MidStreamFallback(_) => "MidStreamFallbackError",
        ErrorClass::Unclassified(_) => "Exception",
    }
}

// A plain Python Exception has no status_code; every class derived from openai's errors carries one
#[must_use]
pub fn class_status(class: &ErrorClass) -> Option<u16> {
    if matches!(class, ErrorClass::Unclassified(_)) {
        return None;
    }
    Some(class.metadata().status_code)
}
