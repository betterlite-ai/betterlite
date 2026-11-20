use std::collections::BTreeMap;

use betterlite_config::sections::retry_policy::RetryPolicySettings;
use betterlite_error::error::retry_policy::{RetryFamily, RetryPolicy as SharedRetryPolicy};
use serde::{Deserialize, Serialize};

use crate::cooldown_decisions::failure::FailureClass;

// LiteLLM names these fields on the config wire, so the serde names are part of compatibility
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

impl From<&RetryPolicySettings> for RetryPolicy {
    fn from(policy: &RetryPolicySettings) -> Self {
        Self {
            bad_request: policy.bad_request,
            authentication: policy.authentication,
            timeout: policy.timeout,
            rate_limit: policy.rate_limit,
            content_policy: policy.content_policy,
            internal_server: policy.internal_server,
            service_unavailable: policy.service_unavailable,
            not_found: policy.not_found,
            fallback: policy.fallback,
        }
    }
}

// a failure class names the retry slot the shared policy is keyed on
impl From<FailureClass> for RetryFamily {
    fn from(class: FailureClass) -> Self {
        match class {
            FailureClass::Authentication => Self::Authentication,
            FailureClass::Timeout => Self::Timeout,
            FailureClass::RateLimit => Self::RateLimit,
            FailureClass::ContentPolicyViolation => Self::ContentPolicyViolation,
            FailureClass::BadRequest
            | FailureClass::ContextWindowExceeded
            | FailureClass::UnsupportedParams => Self::BadRequest,
            FailureClass::ServiceUnavailable | FailureClass::MidStreamFallback => {
                Self::ServiceUnavailable
            }
            FailureClass::InternalServer => Self::InternalServer,
            FailureClass::NotFound
            | FailureClass::PermissionDenied
            | FailureClass::UnprocessableEntity
            | FailureClass::BudgetExceeded
            | FailureClass::BadGateway
            | FailureClass::Api
            | FailureClass::ApiConnection
            | FailureClass::ApiResponseValidation
            | FailureClass::JsonSchemaValidation
            | FailureClass::GuardrailRaised
            | FailureClass::Unclassified => Self::None,
        }
    }
}

impl RetryPolicy {
    // the config wire names the counts; LiteLLM checks a 404 answer before the exception's class, and the shared policy owns that order
    #[must_use]
    pub fn retries_for(&self, class: FailureClass, status: Option<u16>) -> Option<i64> {
        let counts = SharedRetryPolicy {
            bad_request: self.bad_request,
            authentication: self.authentication,
            timeout: self.timeout,
            rate_limit: self.rate_limit,
            content_policy: self.content_policy,
            internal_server: self.internal_server,
            service_unavailable: self.service_unavailable,
            not_found: self.not_found,
            fallback: self.fallback,
        };
        counts.retries_for(RetryFamily::from(class), status)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryPolicies {
    pub global: Option<RetryPolicy>,
    pub model_group: BTreeMap<String, RetryPolicy>,
}

impl RetryPolicies {
    // a model group's policy replaces the global one whole: unset fields never fall back to it
    #[must_use]
    pub fn selected(&self, model_group: Option<&str>) -> Option<&RetryPolicy> {
        model_group
            .and_then(|group| self.model_group.get(group))
            .or(self.global.as_ref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryCounts {
    pub request: Option<i64>,
    pub router: i64,
    pub deployment: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPlan {
    pub num_retries: i64,
    pub policy_applies: bool,
}

// the caller's request, a failed deployment and the retry policy decide the count before the first retry
#[must_use]
pub fn plan(
    counts: RetryCounts,
    policies: &RetryPolicies,
    class: FailureClass,
    status: Option<u16>,
    model_group: Option<&str>,
) -> RetryPlan {
    let num_retries = if counts.request.is_none()
        && let Some(deployment) = counts.deployment
    {
        i64::from(deployment)
    } else {
        counts.request.unwrap_or(counts.router)
    };
    if counts.request == Some(0) {
        return RetryPlan {
            num_retries,
            policy_applies: false,
        };
    }
    let configured = policies
        .selected(model_group)
        .and_then(|policy| policy.retries_for(class, status));
    let Some(retries) = configured else {
        return RetryPlan {
            num_retries,
            policy_applies: false,
        };
    };
    RetryPlan {
        num_retries: retries,
        policy_applies: true,
    }
}
