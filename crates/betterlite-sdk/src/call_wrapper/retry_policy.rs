use betterlite_error::error::ErrorClass;
use serde::Deserialize;

// LiteLLM's litellm/types/router.py RetryPolicy, read off the request's retry_policy object
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetryPolicy {
    #[serde(rename = "BadRequestErrorRetries")]
    pub(crate) bad_request: Option<i64>,
    #[serde(rename = "AuthenticationErrorRetries")]
    pub(crate) authentication: Option<i64>,
    #[serde(rename = "TimeoutErrorRetries")]
    pub(crate) timeout: Option<i64>,
    #[serde(rename = "RateLimitErrorRetries")]
    pub(crate) rate_limit: Option<i64>,
    #[serde(rename = "ContentPolicyViolationErrorRetries")]
    pub(crate) content_policy: Option<i64>,
    #[serde(rename = "InternalServerErrorRetries")]
    pub(crate) internal_server: Option<i64>,
    #[serde(rename = "ServiceUnavailableErrorRetries")]
    pub(crate) service_unavailable: Option<i64>,
    #[serde(rename = "NotFoundErrorRetries")]
    pub(crate) not_found: Option<i64>,
    #[serde(rename = "DefaultRetries")]
    pub(crate) fallback: Option<i64>,
}

impl RetryPolicy {
    // get_num_retries_from_retry_policy: a 404 answer takes NotFoundErrorRetries, then the MRO most specific first
    #[must_use]
    pub(crate) fn retries_for(&self, class: &ErrorClass, status: Option<u16>) -> Option<i64> {
        if status == Some(404)
            && let Some(retries) = self.not_found
        {
            return Some(retries);
        }
        if let Some(retries) = self.by_class(class).into_iter().flatten().next() {
            return Some(retries);
        }
        self.fallback
    }

    // The MRO walk up to the next named exception: a content policy violation and a context window error are bad requests
    const fn by_class(&self, class: &ErrorClass) -> [Option<i64>; 2] {
        match class {
            ErrorClass::Authentication(_) => [self.authentication, None],
            ErrorClass::Timeout(_) => [self.timeout, None],
            ErrorClass::RateLimit(_) => [self.rate_limit, None],
            ErrorClass::ContentPolicyViolation(_) => [self.content_policy, self.bad_request],
            ErrorClass::BadRequest(_)
            | ErrorClass::ContextWindowExceeded(_)
            | ErrorClass::UnsupportedParams(_) => [self.bad_request, None],
            ErrorClass::ServiceUnavailable(_) | ErrorClass::MidStreamFallback(_) => {
                [self.service_unavailable, None]
            }
            ErrorClass::InternalServer(_) => [self.internal_server, None],
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
            | ErrorClass::Unclassified(_) => [None, None],
        }
    }
}
