use std::ops::Div as _;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::payload::StandardLoggingPayload;

use super::error::MetricsError;
use super::series::{
    COUNT_TYPE, DatadogMetricPoint, DatadogMetricSeries, GAUGE_TYPE, OVERHEAD_LATENCY_METRIC,
    PROVIDER_LATENCY_METRIC, REQUEST_COUNT_METRIC, TOTAL_LATENCY_METRIC,
};
use super::settings::MetricsEnvironment;
use super::tags::extract_tags;

pub const MILLIS_PER_SECOND: f64 = 1000.0;
pub const SUCCESS_STATUS_CODE: &str = "200";
pub const FAILED_STATUS_CODE: &str = "500";

// the kwargs LiteLLM's async event handlers receive, as epoch seconds
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestTiming {
    pub start_time: Option<Decimal>,
    pub end_time: Option<Decimal>,
    pub api_call_start_time: Option<Decimal>,
}

pub struct MetricSource<'source> {
    pub environment: &'source MetricsEnvironment,
    pub payload: &'source StandardLoggingPayload,
    pub timing: RequestTiming,
    pub status_code: String,
}

pub fn metrics_from_log(
    source: &MetricSource<'_>,
    flush_interval: u64,
    now: Decimal,
) -> Result<Vec<DatadogMetricSeries>, MetricsError> {
    let end_time = source.timing.end_time.unwrap_or(now);
    let timestamp = end_time
        .to_i64()
        .ok_or(MetricsError::Timestamp { seconds: end_time })?;
    let tags = extract_tags(
        source.environment,
        source.payload,
        Some(source.status_code.as_str()),
    );
    let mut series = Vec::new();
    let latencies = [
        (source.timing.start_time, TOTAL_LATENCY_METRIC, "total"),
        (
            source.timing.api_call_start_time,
            PROVIDER_LATENCY_METRIC,
            "provider",
        ),
    ];
    for (maybe_start, metric, duration) in latencies {
        if let Some(start_time) = maybe_start {
            let value = duration_seconds(end_time, start_time, duration)?;
            series.push(gauge(metric, timestamp, value, tags.clone()));
        }
    }
    if let Some(overhead) = overhead_series(source, timestamp)? {
        series.push(overhead);
    }
    series.push(DatadogMetricSeries {
        metric: REQUEST_COUNT_METRIC,
        kind: COUNT_TYPE,
        points: vec![DatadogMetricPoint {
            timestamp,
            value: 1.0_f64,
        }],
        tags,
        interval: Some(flush_interval),
    });
    Ok(series)
}

#[must_use]
pub fn failure_status_code(payload: &StandardLoggingPayload) -> String {
    payload
        .error_information
        .as_ref()
        .and_then(|information| information.error_code.clone())
        .flatten()
        .unwrap_or_else(|| FAILED_STATUS_CODE.to_owned())
}

fn overhead_series(
    source: &MetricSource<'_>,
    timestamp: i64,
) -> Result<Option<DatadogMetricSeries>, MetricsError> {
    let Some(overhead) = source.payload.hidden_params.litellm_overhead_time_ms else {
        return Ok(None);
    };
    let millis = overhead
        .0
        .to_string()
        .parse::<f64>()
        .map_err(MetricsError::Value)?;
    // LiteLLM deliberately leaves status_code off the overhead series
    let tags = extract_tags(source.environment, source.payload, None);
    Ok(Some(gauge(
        OVERHEAD_LATENCY_METRIC,
        timestamp,
        millis.div(MILLIS_PER_SECOND),
        tags,
    )))
}

fn gauge(
    metric: &'static str,
    timestamp: i64,
    value: f64,
    tags: Vec<String>,
) -> DatadogMetricSeries {
    DatadogMetricSeries {
        metric,
        kind: GAUGE_TYPE,
        points: vec![DatadogMetricPoint { timestamp, value }],
        tags,
        interval: None,
    }
}

fn duration_seconds(
    end_time: Decimal,
    start_time: Decimal,
    duration: &'static str,
) -> Result<f64, MetricsError> {
    let elapsed = end_time
        .checked_sub(start_time)
        .ok_or(MetricsError::Duration { duration })?;
    elapsed
        .to_string()
        .parse::<f64>()
        .map_err(MetricsError::Value)
}
