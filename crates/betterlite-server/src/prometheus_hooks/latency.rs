use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::prometheus_exporter::Handle;
use betterlite_logging::prometheus_latency::latency_histograms::{
    LatencyHistogram, LatencyHistograms, MetricHandle,
};
use chrono::Utc;
use rust_decimal::Decimal;

use super::hooks::PrometheusHooks;

// LiteLLM reads kwargs["completion_start_time"], and a payload builder that lacked one leaves zero there
pub(crate) fn completion_start_time(payload: &StandardLoggingPayload) -> Option<Decimal> {
    (payload.completion_start_time != Decimal::ZERO).then_some(payload.completion_start_time)
}

pub(crate) fn now_seconds() -> Decimal {
    let millis = Utc::now().timestamp_millis();
    Decimal::from(millis)
        .checked_div(Decimal::ONE_THOUSAND)
        .unwrap_or_default()
}

impl PrometheusHooks {
    pub(crate) fn latency_histograms(&self) -> LatencyHistograms<'_, Handle> {
        LatencyHistograms {
            time_to_first_token: self.histogram(
                &self.collectors.litellm_llm_api_time_to_first_token_metric,
                "litellm_llm_api_time_to_first_token_metric",
            ),
            api_latency: self.histogram(
                &self.collectors.litellm_llm_api_latency_metric,
                "litellm_llm_api_latency_metric",
            ),
            total_latency: self.histogram(
                &self.collectors.litellm_request_total_latency_metric,
                "litellm_request_total_latency_metric",
            ),
            queue_time: self.histogram(
                &self.collectors.litellm_request_queue_time_metric,
                "litellm_request_queue_time_seconds",
            ),
        }
    }

    fn histogram<'histograms>(
        &'histograms self,
        metric: &'histograms Handle,
        metric_name: &'static str,
    ) -> LatencyHistogram<'histograms, Handle> {
        LatencyHistogram {
            handle: MetricHandle {
                metric,
                supported_labels: self
                    .collectors
                    .cached_labels
                    .get(metric_name)
                    .map_or(&[], Vec::as_slice),
            },
            metric_name,
            tracker: &self.collectors.bounded_series_tracker,
            limits: self.settings.limits,
        }
    }
}
