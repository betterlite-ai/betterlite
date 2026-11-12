use std::collections::BTreeMap;

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

impl RetryPolicy {
    // LiteLLM checks a 404 answer before the exception's class, then walks its MRO most specific first
    #[must_use]
    pub fn retries_for(&self, class: FailureClass, status: Option<u16>) -> Option<i64> {
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

    const fn by_class(&self, class: FailureClass) -> [Option<i64>; 2] {
        match class {
            FailureClass::Authentication => [self.authentication, None],
            FailureClass::Timeout => [self.timeout, None],
            FailureClass::RateLimit => [self.rate_limit, None],
            FailureClass::ContentPolicyViolation => [self.content_policy, self.bad_request],
            FailureClass::BadRequest
            | FailureClass::ContextWindowExceeded
            | FailureClass::UnsupportedParams => [self.bad_request, None],
            FailureClass::ServiceUnavailable | FailureClass::MidStreamFallback => {
                [self.service_unavailable, None]
            }
            FailureClass::InternalServer => [self.internal_server, None],
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
            | FailureClass::Unclassified => [None, None],
        }
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
