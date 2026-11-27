use betterlite_logging::prometheus_deployment::metric::DeploymentMetric;
use betterlite_logging::prometheus_exporter::{Handle, PrometheusCollectors};

pub(super) fn deployment_handle(
    collectors: &PrometheusCollectors,
    metric: DeploymentMetric,
) -> &Handle {
    match metric {
        DeploymentMetric::CallbackLoggingFailures => {
            &collectors.litellm_callback_logging_failures_metric // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::CooledDown => &collectors.litellm_deployment_cooled_down,
        DeploymentMetric::DeploymentState => &collectors.litellm_deployment_state,
        DeploymentMetric::FailedFallbacks => &collectors.litellm_deployment_failed_fallbacks, // ast-grep-ignore: no-long-names
        DeploymentMetric::FailureResponses => {
            &collectors.litellm_deployment_failure_responses // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::LatencyPerOutputToken => {
            &collectors.litellm_deployment_latency_per_output_token // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::ManagedBatchCreated => {
            &collectors.litellm_managed_batch_created_total // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::OverheadLatency => {
            &collectors.litellm_overhead_latency_metric // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::OverheadWithGuardrails => {
            &collectors.litellm_overhead_with_guardrails_latency_metric // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::RemainingRequests => {
            &collectors.litellm_remaining_requests_metric // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::RemainingTokens => {
            &collectors.litellm_remaining_tokens_metric // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::SuccessResponses => {
            &collectors.litellm_deployment_success_responses // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::SuccessfulFallbacks => {
            &collectors.litellm_deployment_successful_fallbacks // ast-grep-ignore: no-long-names
        }
        DeploymentMetric::TotalRequests => {
            &collectors.litellm_deployment_total_requests // ast-grep-ignore: no-long-names
        }
    }
}
