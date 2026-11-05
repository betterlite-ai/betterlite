use crate::error::ErrorClass;
use crate::rate_limit::RateLimitDimension;

#[derive(Clone, Copy, Default, serde::Deserialize)]
pub enum Class {
    BudgetExceededError,
    RateLimitError,
    AuthenticationError,
    PermissionDeniedError,
    ContextWindowExceededError,
    ContentPolicyViolationError,
    GuardrailRaisedException,
    BlockedPiiEntityError,
    UnsupportedParamsError,
    NotFoundError,
    Timeout,
    MidStreamFallbackError,
    APIConnectionError,
    ServiceUnavailableError,
    InternalServerError,
    BadGatewayError,
    BadRequestError,
    UnprocessableEntityError,
    NotImplementedError,
    TypeError,
    KeyError,
    AttributeError,
    IndexError,
    RuntimeError,
    AssertionError,
    ZeroDivisionError,
    #[default]
    Other,
}

impl Class {
    pub(super) fn code(self, dimension: Option<RateLimitDimension>) -> Option<&'static str> {
        match self {
            Self::BudgetExceededError => Some("429_BUDGET_EXCEEDED"),
            Self::RateLimitError if dimension == Some(RateLimitDimension::Budget) => {
                Some("429_BUDGET_EXCEEDED")
            }
            Self::RateLimitError => Some("429_RATE_LIMIT_EXCEEDED"),
            Self::AuthenticationError => Some("401_AUTHENTICATION_FAILED"),
            Self::PermissionDeniedError => Some("403_PERMISSION_DENIED"),
            Self::ContextWindowExceededError => Some("400_CONTEXT_WINDOW_EXCEEDED"),
            Self::ContentPolicyViolationError
            | Self::GuardrailRaisedException
            | Self::BlockedPiiEntityError => Some("400_CONTENT_POLICY_VIOLATION"),
            Self::UnsupportedParamsError => Some("400_INVALID_PARAMETER_VALUE"),
            Self::NotFoundError => Some("404_RESOURCE_NOT_FOUND"),
            Self::Timeout => Some("408_UPSTREAM_TIMEOUT"),
            Self::MidStreamFallbackError => Some("500_ROUTER_FALLBACK_FAILURE"),
            Self::APIConnectionError => Some("500_PROVIDER_CONNECTION_ERROR"),
            Self::ServiceUnavailableError => Some("503_PROVIDER_OVERLOADED"),
            Self::InternalServerError | Self::BadGatewayError => {
                Some("500_PROVIDER_INTERNAL_ERROR")
            }
            Self::BadRequestError | Self::UnprocessableEntityError => Some("400_INVALID_REQUEST"),
            Self::NotImplementedError => Some("500_UNSUPPORTED_OPERATION"),
            Self::TypeError
            | Self::KeyError
            | Self::AttributeError
            | Self::IndexError
            | Self::RuntimeError
            | Self::AssertionError
            | Self::ZeroDivisionError => Some("500_INTERNAL_STATE_ERROR"),
            Self::Other => None,
        }
    }
}

impl From<&ErrorClass> for Class {
    fn from(error: &ErrorClass) -> Self {
        match error {
            ErrorClass::Authentication(_) => Self::AuthenticationError,
            ErrorClass::NotFound(_) => Self::NotFoundError,
            ErrorClass::PermissionDenied(_) => Self::PermissionDeniedError,
            ErrorClass::BadRequest(_) => Self::BadRequestError,
            ErrorClass::UnprocessableEntity(_) => Self::UnprocessableEntityError,
            ErrorClass::Timeout(_) => Self::Timeout,
            ErrorClass::RateLimit(_) => Self::RateLimitError,
            ErrorClass::BudgetExceeded(_) => Self::BudgetExceededError,
            ErrorClass::ContextWindowExceeded(_) => Self::ContextWindowExceededError,
            ErrorClass::ContentPolicyViolation(_) => Self::ContentPolicyViolationError,
            ErrorClass::ServiceUnavailable(_) => Self::ServiceUnavailableError,
            ErrorClass::BadGateway(_) => Self::BadGatewayError,
            ErrorClass::InternalServer(_) => Self::InternalServerError,
            ErrorClass::ApiConnection(_) => Self::APIConnectionError,
            ErrorClass::UnsupportedParams(_) => Self::UnsupportedParamsError,
            ErrorClass::GuardrailRaised(_) => Self::GuardrailRaisedException,
            ErrorClass::MidStreamFallback(_) => Self::MidStreamFallbackError,
            ErrorClass::Api(_)
            | ErrorClass::ApiResponseValidation(_)
            | ErrorClass::JsonSchemaValidation(_)
            | ErrorClass::Unclassified(_) => Self::Other,
        }
    }
}

mod tests;
