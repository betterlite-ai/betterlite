use serde::{Deserialize, Serialize};

use super::ErrorClass;

// LiteLLM's litellm/types/router.py RetryPolicy, on the router config wire and on a request's retry_policy object
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    #[serde(rename = "BadRequestErrorRetries")]
    pub bad_request: Option<i64>,
    #[serde(rename = "AuthenticationErrorRetries")]
    pub authentication: Option<i64>,
    #[serde(rename = "TimeoutErrorRetries")]
    pub timeout: Option<i64>,
    #[serde(rename = "RateLimitErrorRetries")]
    pub rate_limit: Option<i64>,
    #[serde(rename = "ContentPolicyViolationErrorRetries")]
    pub content_policy: Option<i64>,
    #[serde(rename = "InternalServerErrorRetries")]
    pub internal_server: Option<i64>,
    #[serde(rename = "ServiceUnavailableErrorRetries")]
    pub service_unavailable: Option<i64>,
    #[serde(rename = "NotFoundErrorRetries")]
    pub not_found: Option<i64>,
    #[serde(rename = "DefaultRetries")]
    pub fallback: Option<i64>,
}

// the count a failure takes: LiteLLM walks its exception's MRO up to the next exception the policy names
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryFamily {
    Authentication,
    Timeout,
    RateLimit,
    ContentPolicyViolation,
    BadRequest,
    ServiceUnavailable,
    InternalServer,
    None,
}

// a failure family answers with the retry slot the policy is keyed on
impl From<&ErrorClass> for RetryFamily {
    fn from(class: &ErrorClass) -> Self {
        match class {
            ErrorClass::Authentication(_) => Self::Authentication,
            ErrorClass::Timeout(_) => Self::Timeout,
            ErrorClass::RateLimit(_) => Self::RateLimit,
            ErrorClass::ContentPolicyViolation(_) => Self::ContentPolicyViolation,
            ErrorClass::BadRequest(_)
            | ErrorClass::ContextWindowExceeded(_)
            | ErrorClass::UnsupportedParams(_) => Self::BadRequest,
            ErrorClass::ServiceUnavailable(_) | ErrorClass::MidStreamFallback(_) => {
                Self::ServiceUnavailable
            }
            ErrorClass::InternalServer(_) => Self::InternalServer,
            ErrorClass::NotFound(_)
            | ErrorClass::PermissionDenied(_)
            | ErrorClass::UnprocessableEntity(_)
            | ErrorClass::BudgetExceeded(_)
            | ErrorClass::BadGateway(_)
            | ErrorClass::Api(_)
            | ErrorClass::ApiConnection(_)
            | ErrorClass::ApiResponseValidation(_)
            | ErrorClass::JsonSchemaValidation(_)
            | ErrorClass::GuardrailRaised(_)
            | ErrorClass::Unclassified(_) => Self::None,
        }
    }
}

impl RetryPolicy {
    // get_num_retries_from_retry_policy: a 404 answer takes NotFoundErrorRetries, then the MRO most specific first
    #[must_use]
    pub fn retries_for(&self, family: RetryFamily, status: Option<u16>) -> Option<i64> {
        if status == Some(404)
            && let Some(retries) = self.not_found
        {
            return Some(retries);
        }
        if let Some(retries) = self.by_class(family).into_iter().flatten().next() {
            return Some(retries);
        }
        self.fallback
    }

    const fn by_class(&self, family: RetryFamily) -> [Option<i64>; 2] {
        match family {
            RetryFamily::Authentication => [self.authentication, None],
            RetryFamily::Timeout => [self.timeout, None],
            RetryFamily::RateLimit => [self.rate_limit, None],
            RetryFamily::ContentPolicyViolation => [self.content_policy, self.bad_request],
            RetryFamily::BadRequest => [self.bad_request, None],
            RetryFamily::ServiceUnavailable => [self.service_unavailable, None],
            RetryFamily::InternalServer => [self.internal_server, None],
            RetryFamily::None => [None, None],
        }
    }
}

mod tests;
