use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_exporter::{Handle, PrometheusCollectors};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_rate_limits::gauge::RateLimitGauges;
use crate::prometheus_rate_limits::metric::RateLimitMetric;

use super::metric::DeploymentMetric;
use super::metrics::DeploymentMetrics;

// the registered collectors one write path uses, with the label settings and series limits it needs
pub struct DeploymentCollectors<'collectors> {
    pub collectors: &'collectors PrometheusCollectors,
    pub settings: &'collectors PrometheusLabelSettings,
    pub end_user_settings: EndUserCostTrackingSettings,
    pub limits: SeriesLimits,
}

impl<'collectors> DeploymentCollectors<'collectors> {
    #[must_use]
    pub const fn new(
        collectors: &'collectors PrometheusCollectors,
        settings: &'collectors PrometheusLabelSettings,
        end_user_settings: EndUserCostTrackingSettings,
        limits: SeriesLimits,
    ) -> Self {
        Self {
            collectors,
            settings,
            end_user_settings,
            limits,
        }
    }
}

impl RateLimitGauges for DeploymentCollectors<'_> {
    type Gauge = Handle;

    fn gauge(&self, metric: RateLimitMetric) -> Handle {
        self.collectors.gauge(metric)
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        self.collectors.supported_labels(metric)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        self.collectors.tracker()
    }
}

impl DeploymentMetrics for DeploymentCollectors<'_> {
    fn metric(&self, metric: DeploymentMetric) -> Handle {
        match metric {
            DeploymentMetric::CallbackLoggingFailures => {
                // ast-grep-ignore: no-long-names
                self.collectors
                    .litellm_callback_logging_failures_metric
                    .clone()
            }
            DeploymentMetric::CooledDown => self.collectors.litellm_deployment_cooled_down.clone(),
            DeploymentMetric::DeploymentState => self.collectors.litellm_deployment_state.clone(),
            DeploymentMetric::FailedFallbacks => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_deployment_failed_fallbacks.clone()
            }
            DeploymentMetric::FailureResponses => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_deployment_failure_responses.clone()
            }
            DeploymentMetric::LatencyPerOutputToken => {
                // ast-grep-ignore: no-long-names
                self.collectors
                    .litellm_deployment_latency_per_output_token
                    .clone()
            }
            DeploymentMetric::ManagedBatchCreated => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_managed_batch_created_total.clone()
            }
            DeploymentMetric::OverheadLatency => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_overhead_latency_metric.clone()
            }
            DeploymentMetric::OverheadWithGuardrails => {
                // ast-grep-ignore: no-long-names
                self.collectors
                    .litellm_overhead_with_guardrails_latency_metric
                    .clone()
            }
            DeploymentMetric::RemainingRequests => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_remaining_requests_metric.clone()
            }
            DeploymentMetric::RemainingTokens => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_remaining_tokens_metric.clone()
            }
            DeploymentMetric::SuccessResponses => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_deployment_success_responses.clone()
            }
            DeploymentMetric::SuccessfulFallbacks => {
                // ast-grep-ignore: no-long-names
                self.collectors
                    .litellm_deployment_successful_fallbacks
                    .clone()
            }
            DeploymentMetric::TotalRequests => {
                // ast-grep-ignore: no-long-names
                self.collectors.litellm_deployment_total_requests.clone()
            }
        }
    }

    fn labels_for(&self, metric: DeploymentMetric) -> &[String] {
        self.collectors
            .cached_labels
            .get(metric.name())
            .map_or(&[], Vec::as_slice)
    }

    fn settings(&self) -> &PrometheusLabelSettings {
        self.settings
    }

    fn end_user_settings(&self) -> EndUserCostTrackingSettings {
        self.end_user_settings
    }

    fn limits(&self) -> &SeriesLimits {
        &self.limits
    }
}
