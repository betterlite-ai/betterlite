use betterlite_prometheus::collector_factory::collector::Collector;

use crate::prometheus_exporter::{Handle, PrometheusCollectors};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric,
};

use super::metric::RateLimitMetric;

// the gauges the rate-limit emitters write, so a test can record them without a registry and B8655 hands in the real collectors
pub trait RateLimitGauges {
    type Gauge: Collector + RemovableMetric;

    fn gauge(&self, metric: RateLimitMetric) -> Self::Gauge;
    fn supported_labels(&self, metric: RateLimitMetric) -> &[String];
    fn tracker(&self) -> &BoundedPrometheusSeriesTracker;
}

impl RateLimitGauges for PrometheusCollectors {
    type Gauge = Handle;

    fn gauge(&self, metric: RateLimitMetric) -> Handle {
        match metric {
            RateLimitMetric::ApiKeyRateLimitAllowed => {
                // ast-grep-ignore: no-long-names
                self.litellm_api_key_rate_limit_allowed_metric.clone()
            }
            RateLimitMetric::ApiKeyRateLimitUsed => {
                // ast-grep-ignore: no-long-names
                self.litellm_api_key_rate_limit_used_metric.clone()
            }
            RateLimitMetric::TeamRateLimitAllowed => {
                // ast-grep-ignore: no-long-names
                self.litellm_team_rate_limit_allowed_metric.clone()
            }
            // ast-grep-ignore: no-long-names
            RateLimitMetric::TeamRateLimitUsed => self.litellm_team_rate_limit_used_metric.clone(),
            RateLimitMetric::RemainingApiKeyRequests => {
                // ast-grep-ignore: no-long-names
                self.litellm_remaining_api_key_requests_for_model.clone()
            }
            RateLimitMetric::RemainingApiKeyTokens => {
                // ast-grep-ignore: no-long-names
                self.litellm_remaining_api_key_tokens_for_model.clone()
            }
            RateLimitMetric::DeploymentTpmLimit => self.litellm_deployment_tpm_limit.clone(),
            RateLimitMetric::DeploymentRpmLimit => self.litellm_deployment_rpm_limit.clone(),
            // ast-grep-ignore: no-long-names
            RateLimitMetric::RemainingRequests => self.litellm_remaining_requests_metric.clone(),
            // ast-grep-ignore: no-long-names
            RateLimitMetric::RemainingTokens => self.litellm_remaining_tokens_metric.clone(),
        }
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        self.cached_labels
            .get(metric.name())
            .map_or(&[], Vec::as_slice)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.bounded_series_tracker
    }
}
