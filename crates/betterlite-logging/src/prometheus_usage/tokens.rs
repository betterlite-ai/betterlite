use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::counters::{UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use super::sparse::{SparseValue, inc_sparse_usage_counters};
use super::usage_object::{object, resolve_cache_write_tokens};
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;

pub fn increment_token_metrics<C>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    counters.inc(
        UsageMetric::TotalTokens,
        labels,
        MetricValue::Finite(Decimal::from(payload.total_tokens)),
        now,
    )?;
    counters.inc(
        UsageMetric::InputTokens,
        labels,
        MetricValue::Finite(Decimal::from(payload.prompt_tokens)),
        now,
    )?;
    counters.inc(
        UsageMetric::OutputTokens,
        labels,
        MetricValue::Finite(Decimal::from(payload.completion_tokens)),
        now,
    )?;
    increment_token_detail_metrics(counters, payload, labels, now)
}

pub fn increment_token_detail_metrics<C>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    let Some(usage) = object(payload.metadata.usage_object.as_ref()) else {
        return Ok(());
    };
    let prompt_details = usage.get("prompt_tokens_details");
    let prompt = object(prompt_details);
    let completion = object(usage.get("completion_tokens_details"));
    let cached = prompt.and_then(|details| details.get("cached_tokens"));
    let audio = prompt.and_then(|details| details.get("audio_tokens"));
    let reasoning = completion.and_then(|details| details.get("reasoning_tokens"));
    let output_audio = completion.and_then(|details| details.get("audio_tokens"));
    let write = resolve_cache_write_tokens(prompt_details);
    let entries = [
        (
            UsageMetric::InputCachedTokens,
            cached.map(SparseValue::Json),
        ),
        (
            UsageMetric::InputCacheCreationTokens,
            write.map(SparseValue::Integer),
        ),
        (UsageMetric::InputAudioTokens, audio.map(SparseValue::Json)),
        (
            UsageMetric::OutputReasoningTokens,
            reasoning.map(SparseValue::Json),
        ),
        (
            UsageMetric::OutputAudioTokens,
            output_audio.map(SparseValue::Json),
        ),
    ];
    inc_sparse_usage_counters(counters, &entries, labels, now)
}
