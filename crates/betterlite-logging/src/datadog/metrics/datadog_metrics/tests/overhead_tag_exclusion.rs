use rust_decimal::Decimal;

use crate::datadog::metrics::measurements::RequestTiming;
use crate::datadog::metrics::series::{
    DatadogMetricSeries, OVERHEAD_LATENCY_METRIC, PROVIDER_LATENCY_METRIC, REQUEST_COUNT_METRIC,
    TOTAL_LATENCY_METRIC,
};
use crate::fields::payload::records::DecimalValue;

use super::support;

fn series_for<'series>(
    metric: &str,
    series: &'series [DatadogMetricSeries],
) -> &'series DatadogMetricSeries {
    series
        .iter()
        .find(|entry| entry.metric == metric)
        .expect("the metric series is present")
}

fn has_status_tag(series: &DatadogMetricSeries) -> bool {
    series
        .tags
        .iter()
        .any(|tag| tag.starts_with("status_code:"))
}

#[test]
fn overhead_tag_exclusion_leaves_status_off_the_overhead_series() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    let mut payload = support::payload();
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::new(12_500, 3)));
    let timing = RequestTiming {
        start_time: Some(Decimal::from(1_704_067_200_i64)),
        end_time: Some(Decimal::from(1_704_067_202_i64)),
        api_call_start_time: Some(Decimal::from(1_704_067_201_i64)),
    };
    let series = support::metrics_for(&logger, &payload, "429", timing);
    let overhead = series_for(OVERHEAD_LATENCY_METRIC, &series);
    assert!(!has_status_tag(overhead));
    assert!(has_status_tag(series_for(TOTAL_LATENCY_METRIC, &series)));
    assert!(has_status_tag(series_for(PROVIDER_LATENCY_METRIC, &series)));
    assert!(has_status_tag(series_for(REQUEST_COUNT_METRIC, &series)));
}

#[test]
fn overhead_tag_exclusion_keeps_the_count_status_tag() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    let mut payload = support::payload();
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::from(7_i64)));
    let series = support::metrics_for(
        &logger,
        &payload,
        "200",
        RequestTiming {
            start_time: Some(payload.start_time),
            end_time: Some(payload.end_time),
            api_call_start_time: None,
        },
    );
    let count = series_for(REQUEST_COUNT_METRIC, &series);
    assert!(count.tags.contains(&"status_code:200".to_owned()));
    let overhead = series_for(OVERHEAD_LATENCY_METRIC, &series);
    assert!(overhead.tags.contains(&"model_name:gpt-4o".to_owned()));
}
