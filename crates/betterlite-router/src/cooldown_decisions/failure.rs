use betterlite_error::error::ErrorClass;
use serde::{Deserialize, Serialize};

// LiteLLM compares float seconds and the probe protocol's operation derives Eq, so the value carries its own equality
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seconds(pub f64);

impl Eq for Seconds {}

// LiteLLM reads exception.status_code as an int when it has one and as "" when it does not
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorStatus {
    Code(i64),
    Text(String),
}

impl ErrorStatus {
    pub(super) fn cast_code(&self) -> i64 {
        match self {
            Self::Code(code) => *code,
            Self::Text(text) => text.trim().parse().unwrap_or(500),
        }
    }
}

// closed mirror of betterlite's typed B011 families, named for the LiteLLM exception the family stands for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Authentication,
    NotFound,
    PermissionDenied,
    BadRequest,
    UnprocessableEntity,
    Timeout,
    RateLimit,
    BudgetExceeded,
    ContextWindowExceeded,
    ContentPolicyViolation,
    ServiceUnavailable,
    BadGateway,
    InternalServer,
    Api,
    ApiConnection,
    ApiResponseValidation,
    JsonSchemaValidation,
    UnsupportedParams,
    GuardrailRaised,
    MidStreamFallback,
    Unclassified,
}

impl From<&ErrorClass> for FailureClass {
    fn from(class: &ErrorClass) -> Self {
        match class {
            ErrorClass::Authentication(_) => Self::Authentication,
            ErrorClass::NotFound(_) => Self::NotFound,
            ErrorClass::PermissionDenied(_) => Self::PermissionDenied,
            ErrorClass::BadRequest(_) => Self::BadRequest,
            ErrorClass::UnprocessableEntity(_) => Self::UnprocessableEntity,
            ErrorClass::Timeout(_) => Self::Timeout,
            ErrorClass::RateLimit(_) => Self::RateLimit,
            ErrorClass::BudgetExceeded(_) => Self::BudgetExceeded,
            ErrorClass::ContextWindowExceeded(_) => Self::ContextWindowExceeded,
            ErrorClass::ContentPolicyViolation(_) => Self::ContentPolicyViolation,
            ErrorClass::ServiceUnavailable(_) => Self::ServiceUnavailable,
            ErrorClass::BadGateway(_) => Self::BadGateway,
            ErrorClass::InternalServer(_) => Self::InternalServer,
            ErrorClass::Api(_) => Self::Api,
            ErrorClass::ApiConnection(_) => Self::ApiConnection,
            ErrorClass::ApiResponseValidation(_) => Self::ApiResponseValidation,
            ErrorClass::JsonSchemaValidation(_) => Self::JsonSchemaValidation,
            ErrorClass::UnsupportedParams(_) => Self::UnsupportedParams,
            ErrorClass::GuardrailRaised(_) => Self::GuardrailRaised,
            ErrorClass::MidStreamFallback(_) => Self::MidStreamFallback,
            ErrorClass::Unclassified(_) => Self::Unclassified,
        }
    }
}

// type(exception).__name__, which the allowed-fails cache key carries for a matched policy entry
pub(super) const fn exception_class_name(class: FailureClass) -> &'static str {
    match class {
        FailureClass::Authentication => "AuthenticationError",
        FailureClass::NotFound => "NotFoundError",
        FailureClass::PermissionDenied => "PermissionDeniedError",
        FailureClass::BadRequest => "BadRequestError",
        FailureClass::UnprocessableEntity => "UnprocessableEntityError",
        FailureClass::Timeout => "Timeout",
        FailureClass::RateLimit => "RateLimitError",
        FailureClass::BudgetExceeded => "BudgetExceededError",
        FailureClass::ContextWindowExceeded => "ContextWindowExceededError",
        FailureClass::ContentPolicyViolation => "ContentPolicyViolationError",
        FailureClass::ServiceUnavailable => "ServiceUnavailableError",
        FailureClass::BadGateway => "BadGatewayError",
        FailureClass::InternalServer => "InternalServerError",
        FailureClass::Api => "APIError",
        FailureClass::ApiConnection => "APIConnectionError",
        FailureClass::ApiResponseValidation => "APIResponseValidationError",
        FailureClass::JsonSchemaValidation => "JSONSchemaValidationError",
        FailureClass::UnsupportedParams => "UnsupportedParamsError",
        FailureClass::GuardrailRaised => "GuardrailRaisedException",
        FailureClass::MidStreamFallback => "MidStreamFallbackError",
        FailureClass::Unclassified => "Exception",
    }
}

// model_call_details.litellm_params plus the call's start and end times; None means no litellm_params mapping
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutFacts {
    pub client_side_timeout: bool,
    pub timeout_seconds: Option<Seconds>,
    pub elapsed_seconds: Option<Seconds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureFacts {
    pub class: FailureClass,
    pub status: ErrorStatus,
    pub advisor_orchestration: bool,
    pub background_response_cost_poll: bool,
    // the original exception's family when this failure wraps one, which LiteLLM sees through str(exception)
    #[serde(default)]
    pub wrapped_exception: Option<FailureClass>,
    #[serde(default)]
    pub caller_timeout: Option<TimeoutFacts>,
}

#[must_use]
pub const fn is_background_cost_poll_404(status: &ErrorStatus, origin_is_cost_poll: bool) -> bool {
    if !origin_is_cost_poll {
        return false;
    }
    matches!(status, ErrorStatus::Code(404))
}

#[must_use]
pub fn is_caller_timeout_408(status: &ErrorStatus, timeout_facts: Option<&TimeoutFacts>) -> bool {
    if status.cast_code() != 408 {
        return false;
    }
    let Some(timeout) = timeout_facts else {
        return false;
    };
    if !timeout.client_side_timeout {
        return false;
    }
    match (timeout.timeout_seconds, timeout.elapsed_seconds) {
        (Some(limit), Some(elapsed)) => elapsed.0 >= limit.0,
        _ => false,
    }
}
