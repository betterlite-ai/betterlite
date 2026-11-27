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

mod deployment;
mod usage;

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
        usage::usage_handle(self.collectors, metric)
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
        deployment::deployment_handle(self.collectors, metric).clone()
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
