use std::time::Instant;

use super::counters::{UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use super::sparse::{SparseValue, inc_sparse_usage_counters};
use super::usage_object::usage_object;
use crate::fields::payload::StandardLoggingPayload;

pub fn increment_media_metrics<C: UsageCounters>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<C::Counter::Error>> {
    let Some(usage) = usage_object(payload) else {
        return Ok(());
    };
    let entries = [
        (
            UsageMetric::VideoDurationSeconds,
            usage.get("duration_seconds").map(SparseValue::Json),
        ),
        (
            UsageMetric::ImagesGenerated,
            usage.get("output_image_count").map(SparseValue::Json),
        ),
    ];
    inc_sparse_usage_counters(counters, &entries, labels, now)
}
