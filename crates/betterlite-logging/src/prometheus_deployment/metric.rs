// the PrometheusLogger collectors these outcomes are written to, named after their attributes
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentMetric {
    CallbackLoggingFailures,
    CooledDown,
    DeploymentState,
    FailedFallbacks,
    FailureResponses,
    LatencyPerOutputToken,
    ManagedBatchCreated,
    OverheadLatency,
    OverheadWithGuardrails,
    RemainingRequests,
    RemainingTokens,
    SuccessResponses,
    SuccessfulFallbacks,
    TotalRequests,
}

impl DeploymentMetric {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CallbackLoggingFailures => "litellm_callback_logging_failures_metric",
            Self::CooledDown => "litellm_deployment_cooled_down",
            Self::DeploymentState => "litellm_deployment_state",
            Self::FailedFallbacks => "litellm_deployment_failed_fallbacks",
            Self::FailureResponses => "litellm_deployment_failure_responses",
            Self::LatencyPerOutputToken => "litellm_deployment_latency_per_output_token",
            Self::ManagedBatchCreated => "litellm_managed_batch_created_total",
            Self::OverheadLatency => "litellm_overhead_latency_metric",
            Self::OverheadWithGuardrails => "litellm_overhead_with_guardrails_latency_metric",
            Self::RemainingRequests => "litellm_remaining_requests_metric",
            Self::RemainingTokens => "litellm_remaining_tokens_metric",
            Self::SuccessResponses => "litellm_deployment_success_responses",
            Self::SuccessfulFallbacks => "litellm_deployment_successful_fallbacks",
            Self::TotalRequests => "litellm_deployment_total_requests",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum DeploymentState {
    Healthy = 0,
    PartialOutage = 1,
    CompleteOutage = 2,
}

impl DeploymentState {
    #[must_use]
    pub const fn number(self) -> i64 {
        match self {
            Self::Healthy => 0,
            Self::PartialOutage => 1,
            Self::CompleteOutage => 2,
        }
    }
}
