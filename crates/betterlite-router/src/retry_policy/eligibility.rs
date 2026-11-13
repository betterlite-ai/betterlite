use crate::cooldown_decisions::failure::FailureClass;

// LiteLLM reads the same three fallback lists three different ways, so the counts keep presence and emptiness apart
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Availability {
    pub healthy_deployments: usize,
    pub all_deployments: usize,
    pub context_window_fallbacks: Option<usize>,
    pub content_policy_fallbacks: Option<usize>,
    pub regular_fallbacks: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    Stop(StopReason),
}

// each reason is one raise site in LiteLLM's should_retry_this_error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    ContextWindowFallbacks,
    ContentPolicyFallbacks,
    StatusNotRetryable,
    NotFound,
    RateLimitWithoutHealthy,
    AuthWithoutOtherDeployment,
    NoHealthyDeployments,
}

// LiteLLM retries 401 and 403 only when another deployment exists, which the class checks below decide
const AUTH_STATUSES: [u16; 2] = [401, 403];

#[must_use]
pub fn decide(
    class: FailureClass,
    status: Option<u16>,
    availability: &Availability,
) -> RetryDecision {
    if class == FailureClass::ContextWindowExceeded
        && availability.context_window_fallbacks.is_some()
    {
        return RetryDecision::Stop(StopReason::ContextWindowFallbacks);
    }
    if class == FailureClass::ContentPolicyViolation
        && availability.content_policy_fallbacks.is_some()
    {
        return RetryDecision::Stop(StopReason::ContentPolicyFallbacks);
    }
    if let Some(code) = status
        && !is_retryable_status(code)
        && !AUTH_STATUSES.contains(&code)
    {
        return RetryDecision::Stop(StopReason::StatusNotRetryable);
    }
    if class == FailureClass::NotFound {
        return RetryDecision::Stop(StopReason::NotFound);
    }
    if class == FailureClass::RateLimit
        && availability.healthy_deployments == 0
        && availability
            .regular_fallbacks
            .is_some_and(|count| count > 0)
    {
        return RetryDecision::Stop(StopReason::RateLimitWithoutHealthy);
    }
    if matches!(
        class,
        FailureClass::Authentication | FailureClass::PermissionDenied
    ) && availability.all_deployments <= 1
    {
        return RetryDecision::Stop(StopReason::AuthWithoutOtherDeployment);
    }
    if availability.healthy_deployments > 0 {
        RetryDecision::Retry
    } else {
        RetryDecision::Stop(StopReason::NoHealthyDeployments)
    }
}

// openai's retry set: request and lock timeouts, rate limits, and every server error
pub(crate) const fn is_retryable_status(status: u16) -> bool {
    match status {
        408 | 409 | 429 => true,
        _ => status >= 500,
    }
}
