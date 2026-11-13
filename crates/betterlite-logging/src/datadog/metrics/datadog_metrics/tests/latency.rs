use std::sync::Arc;

use rust_decimal::Decimal;

use crate::datadog::metrics::datadog_metrics::DatadogMetricsLogger;
use crate::datadog::metrics::measurements::RequestTiming;
use crate::datadog::metrics::series::{
    COUNT_TYPE, DatadogMetricPoint, GAUGE_TYPE, OVERHEAD_LATENCY_METRIC, PROVIDER_LATENCY_METRIC,
    REQUEST_COUNT_METRIC, TOTAL_LATENCY_METRIC,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::DecimalValue;

use super::support::{self, NOW};

const START: i64 = 1_704_067_200;
const API_START: i64 = 1_704_067_201;
const END_MILLIS: i64 = 17_040_672_045;

fn timing() -> RequestTiming {
    RequestTiming {
        start_time: Some(Decimal::from(START)),
        end_time: Some(Decimal::new(END_MILLIS, 1)),
        api_call_start_time: Some(Decimal::from(API_START)),
    }
}

fn payload_with_overhead() -> StandardLoggingPayload {
    let mut payload = support::payload();
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::new(12_500, 3)));
    payload
}

fn logger() -> Arc<DatadogMetricsLogger> {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    logger
}

#[test]
fn latency_records_the_total_and_provider_series() {
    let logger = logger();
    let series = support::metrics_for(&logger, &support::payload(), "200", timing());
    assert_eq!(series.len(), 3);
    assert_eq!(series[0].metric, TOTAL_LATENCY_METRIC);
    assert_eq!(series[0].kind, GAUGE_TYPE);
    assert_eq!(
        series[0].points,
        vec![DatadogMetricPoint {
            timestamp: 1_704_067_204,
            value: 4.5,
        }]
    );
    assert_eq!(series[1].metric, PROVIDER_LATENCY_METRIC);
    assert_eq!(series[1].points[0].value.to_bits(), 3.5_f64.to_bits());
    assert_eq!(series[1].points[0].timestamp, 1_704_067_204);
    assert_eq!(series[2].metric, REQUEST_COUNT_METRIC);
    assert_eq!(series[0].interval, None);
    assert_eq!(series[1].interval, None);
}

#[test]
fn latency_records_the_overhead_and_count_series() {
    let logger = logger();
    let series = support::metrics_for(&logger, &payload_with_overhead(), "200", timing());
    assert_eq!(series.len(), 4);
    assert_eq!(series[2].metric, OVERHEAD_LATENCY_METRIC);
    assert_eq!(series[2].points[0].value.to_bits(), 0.0125_f64.to_bits());
    assert_eq!(series[2].points[0].timestamp, 1_704_067_204);
    assert_eq!(series[3].metric, REQUEST_COUNT_METRIC);
    assert_eq!(series[3].kind, COUNT_TYPE);
    assert_eq!(series[3].points[0].value.to_bits(), 1.0_f64.to_bits());
    assert_eq!(series[3].points[0].timestamp, 1_704_067_204);
    assert_eq!(series[3].interval, Some(support::FLUSH_INTERVAL));
    assert_eq!(series[2].interval, None);
}

#[test]
fn latency_skips_the_total_series_without_a_start_time() {
    let logger = logger();
    let timing = RequestTiming {
        start_time: None,
        ..timing()
    };
    let series = support::metrics_for(&logger, &support::payload(), "200", timing);
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].metric, PROVIDER_LATENCY_METRIC);
    assert_eq!(series[1].metric, REQUEST_COUNT_METRIC);
}

#[test]
fn latency_skips_the_provider_series_without_an_api_call_start_time() {
    let logger = logger();
    let timing = RequestTiming {
        api_call_start_time: None,
        ..timing()
    };
    let series = support::metrics_for(&logger, &payload_with_overhead(), "200", timing);
    assert_eq!(series.len(), 3);
    assert_eq!(series[0].metric, TOTAL_LATENCY_METRIC);
    assert_eq!(series[1].metric, OVERHEAD_LATENCY_METRIC);
    assert_eq!(series[2].metric, REQUEST_COUNT_METRIC);
}

#[test]
fn latency_uses_the_clock_for_a_missing_end_time() {
    let logger = logger();
    let timing = RequestTiming {
        start_time: Some(Decimal::from(1_699_999_999_i64)),
        end_time: None,
        api_call_start_time: None,
    };
    let series = support::metrics_for(&logger, &support::payload(), "200", timing);
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].points[0].value.to_bits(), 1.0_f64.to_bits());
    assert_eq!(series[0].points[0].timestamp, NOW);
    assert_eq!(series[1].points[0].timestamp, NOW);
}

#[test]
fn latency_keeps_a_negative_duration() {
    let logger = logger();
    let timing = RequestTiming {
        start_time: Some(Decimal::new(17_000_000_015, 1)),
        end_time: Some(Decimal::from(1_700_000_000_i64)),
        api_call_start_time: None,
    };
    let series = support::metrics_for(&logger, &support::payload(), "200", timing);
    assert_eq!(series[0].points[0].value.to_bits(), (-1.5_f64).to_bits());
}

#[test]
fn latency_keeps_an_overhead_of_zero() {
    let logger = logger();
    let mut payload = support::payload();
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::ZERO));
    let series = support::metrics_for(&logger, &payload, "200", timing());
    assert_eq!(series.len(), 4);
    assert_eq!(series[2].metric, OVERHEAD_LATENCY_METRIC);
    assert_eq!(series[2].points[0].value.to_bits(), 0.0_f64.to_bits());
}

#[test]
fn latency_converts_the_overhead_from_milliseconds() {
    let logger = logger();
    let mut payload = support::payload();
    payload.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(Decimal::from(1_500_i64)));
    let series = support::metrics_for(&logger, &payload, "200", timing());
    assert_eq!(series[2].points[0].value.to_bits(), 1.5_f64.to_bits());
}

#[test]
fn latency_needs_no_api_call_start_for_the_overhead_series() {
    let logger = logger();
    let timing = RequestTiming {
        start_time: None,
        end_time: Some(Decimal::new(END_MILLIS, 1)),
        api_call_start_time: None,
    };
    let series = support::metrics_for(&logger, &payload_with_overhead(), "200", timing);
    assert_eq!(series.len(), 2);
    assert_eq!(series[0].metric, OVERHEAD_LATENCY_METRIC);
    assert_eq!(series[1].metric, REQUEST_COUNT_METRIC);
}
