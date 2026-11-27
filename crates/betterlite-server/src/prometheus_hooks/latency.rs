use betterlite_logging::prometheus_exporter::Handle;
use betterlite_logging::prometheus_latency::latency_histograms::{
    LatencyHistogram, LatencyHistograms, MetricHandle,
};
use chrono::Utc;
use rust_decimal::Decimal;

use super::hooks::PrometheusHooks;

pub(crate) fn now_seconds() -> Decimal {
    let millis = Utc::now().timestamp_millis();
    Decimal::from(millis)
        .checked_div(Decimal::ONE_THOUSAND)
        .unwrap_or_default()
}

pub(crate) fn latency_histograms(hooks: &PrometheusHooks) -> LatencyHistograms<'_, Handle> {
    LatencyHistograms {
        time_to_first_token: histogram(
            hooks,
            &hooks.collectors.litellm_llm_api_time_to_first_token_metric, // ast-grep-ignore: no-long-names
            "litellm_llm_api_time_to_first_token_metric",
        ),
        api_latency: histogram(
            hooks,
            &hooks.collectors.litellm_llm_api_latency_metric,
            "litellm_llm_api_latency_metric",
        ),
        total_latency: histogram(
            hooks,
            &hooks.collectors.litellm_request_total_latency_metric, // ast-grep-ignore: no-long-names
            "litellm_request_total_latency_metric",
        ),
        queue_time: histogram(
            hooks,
            &hooks.collectors.litellm_request_queue_time_metric, // ast-grep-ignore: no-long-names
            "litellm_request_queue_time_seconds",
        ),
    }
}

fn histogram<'histograms>(
    hooks: &'histograms PrometheusHooks,
    metric: &'histograms Handle,
    metric_name: &'static str,
) -> LatencyHistogram<'histograms, Handle> {
    LatencyHistogram {
        handle: MetricHandle {
            metric,
            supported_labels: hooks
                .collectors
                .cached_labels
                .get(metric_name)
                .map_or(&[], Vec::as_slice),
        },
        metric_name,
        tracker: &hooks.collectors.bounded_series_tracker,
        limits: hooks.settings.limits,
    }
}
