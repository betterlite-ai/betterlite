use std::time::Instant;

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::bounded_series::CounterEmission;
use betterlite_logging::prometheus_deployment::metric::DeploymentMetric;
use betterlite_logging::prometheus_deployment::metrics::DeploymentMetrics;
use betterlite_logging::prometheus_exporter::{Handle, PrometheusCollectors};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_logging::prometheus_rate_limits::gauge::RateLimitGauges;
use betterlite_logging::prometheus_rate_limits::metric::RateLimitMetric;
use betterlite_logging::prometheus_usage::counters::UsageCounters;
use betterlite_logging::prometheus_usage::metric::UsageMetric;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use super::error::PrometheusHookError;

// the collectors one hook writes through, with the settings the B3876, B3877, B3878, B3879 and B3881 emitters take
pub(crate) struct HookMetrics<'metrics> {
    pub(crate) collectors: &'metrics PrometheusCollectors,
    settings: &'metrics PrometheusLabelSettings,
    end_user: EndUserCostTrackingSettings,
    limits: &'metrics SeriesLimits,
}

// one labeled counter write: the counter, its registered labels and the timestamp the tracker keeps
pub(crate) struct CounterWrite<'write> {
    pub(crate) counter: &'write Handle,
    pub(crate) metric_name: &'write str,
    pub(crate) values: &'write UserApiKeyLabelValues,
    pub(crate) source: LabelSource<'write>,
    pub(crate) amount: MetricValue,
    pub(crate) now: Instant,
}

impl<'metrics> HookMetrics<'metrics> {
    pub(crate) const fn new(
        collectors: &'metrics PrometheusCollectors,
        settings: &'metrics PrometheusLabelSettings,
        end_user: EndUserCostTrackingSettings,
        limits: &'metrics SeriesLimits,
    ) -> Self {
        Self {
            collectors,
            settings,
            end_user,
            limits,
        }
    }

    fn labels_for(&self, metric_name: &str) -> &[String] {
        self.collectors
            .cached_labels
            .get(metric_name)
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn emit(&self, write: CounterWrite<'_>) -> Result<(), PrometheusHookError> {
        let emission = CounterEmission {
            counter: write.counter,
            metric_name: write.metric_name,
            supported_labels: self.labels_for(write.metric_name),
            tracker: &self.collectors.bounded_series_tracker,
            limits: self.limits,
        };
        emission.inc_labeled_counter(write.values, write.source, write.amount, write.now)
    }
}

impl UsageCounters for HookMetrics<'_> {
    type Counter = Handle;

    fn counter(&self, metric: UsageMetric) -> Handle {
        usage_handle(self.collectors, metric)
    }

    fn supported_labels(&self, metric: UsageMetric) -> &[String] {
        self.labels_for(metric.name())
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.collectors.bounded_series_tracker
    }

    fn limits(&self) -> &SeriesLimits {
        self.limits
    }
}

impl RateLimitGauges for HookMetrics<'_> {
    type Gauge = Handle;

    fn gauge(&self, metric: RateLimitMetric) -> Handle {
        RateLimitGauges::gauge(self.collectors, metric)
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        RateLimitGauges::supported_labels(self.collectors, metric)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        RateLimitGauges::tracker(self.collectors)
    }
}

impl DeploymentMetrics for HookMetrics<'_> {
    fn metric(&self, metric: DeploymentMetric) -> Handle {
        deployment_handle(self.collectors, metric).clone()
    }

    fn labels_for(&self, metric: DeploymentMetric) -> &[String] {
        self.labels_for(metric.name())
    }

    fn settings(&self) -> &PrometheusLabelSettings {
        self.settings
    }

    fn end_user_settings(&self) -> EndUserCostTrackingSettings {
        self.end_user
    }

    fn limits(&self) -> &SeriesLimits {
        self.limits
    }
}

fn usage_handle(collectors: &PrometheusCollectors, metric: UsageMetric) -> Handle {
    match metric {
        UsageMetric::Requests => collectors.litellm_requests_metric.clone(),
        UsageMetric::Spend => collectors.litellm_spend_metric.clone(),
        UsageMetric::ZeroCostRequests => collectors.litellm_zero_cost_requests_total.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::TotalTokens => collectors.litellm_tokens_metric.clone(),
        UsageMetric::InputTokens => collectors.litellm_input_tokens_metric.clone(),
        UsageMetric::OutputTokens => collectors.litellm_output_tokens_metric.clone(),
        UsageMetric::InputCachedTokens => collectors.litellm_input_cached_tokens_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::InputCacheCreationTokens => {
            collectors
                // ast-grep-ignore: no-long-names
                .litellm_input_cache_creation_tokens_metric
                .clone()
        }
        UsageMetric::InputAudioTokens => collectors.litellm_input_audio_tokens_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::OutputReasoningTokens => {
            collectors.litellm_output_reasoning_tokens_metric.clone() // ast-grep-ignore: no-long-names
        }
        UsageMetric::OutputAudioTokens => collectors.litellm_output_audio_tokens_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::VideoDurationSeconds => {
            collectors.litellm_video_duration_seconds_metric.clone() // ast-grep-ignore: no-long-names
        }
        UsageMetric::ImagesGenerated => collectors.litellm_images_generated_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::CacheHits => collectors.litellm_cache_hits_metric.clone(),
        UsageMetric::CacheMisses => collectors.litellm_cache_misses_metric.clone(),
        UsageMetric::CachedTokens => collectors.litellm_cached_tokens_metric.clone(),
        UsageMetric::ProviderCacheReadTokens => {
            collectors
                // ast-grep-ignore: no-long-names
                .litellm_provider_cache_read_input_tokens_metric
                .clone()
        }
        UsageMetric::ProviderCacheCreationTokens => {
            collectors
                // ast-grep-ignore: no-long-names
                .litellm_provider_cache_creation_input_tokens_metric
                .clone()
        }
        UsageMetric::McpToolCalls => collectors.litellm_mcp_tool_calls_total.clone(),
        UsageMetric::McpToolCallSpend => collectors.litellm_mcp_tool_call_spend_metric.clone(), // ast-grep-ignore: no-long-names
        UsageMetric::SpendCaptureRate => collectors.litellm_spend_capture_rate.clone(),
    }
}

fn deployment_handle(collectors: &PrometheusCollectors, metric: DeploymentMetric) -> &Handle {
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
