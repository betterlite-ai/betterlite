use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::counters::{UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use super::usage_object::{object, resolve_provider_cache_tokens};
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;

pub fn increment_cache_metrics<C>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    increment_cache_lookup(counters, payload, labels, now)?;
    increment_provider_cache(
        counters,
        payload.metadata.usage_object.as_ref(),
        labels,
        now,
    )
}

fn increment_cache_lookup<C>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    let Some(cache_hit) = payload.cache_hit else {
        return Ok(());
    };
    let metric = if cache_hit {
        UsageMetric::CacheHits
    } else {
        UsageMetric::CacheMisses
    };
    counters.inc(metric, labels, MetricValue::Finite(Decimal::ONE), now)?;
    if cache_hit && payload.total_tokens > 0 {
        let cached = Decimal::from(payload.total_tokens);
        counters.inc(
            UsageMetric::CachedTokens,
            labels,
            MetricValue::Finite(cached),
            now,
        )?;
    }
    Ok(())
}

fn increment_provider_cache<C>(
    counters: &C,
    usage_object: Option<&Json>,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<<C::Counter as RemovableMetric>::Error>>
where
    C: UsageCounters,
{
    let Some(usage) = object(usage_object) else {
        return Ok(());
    };
    let (read, creation) = resolve_provider_cache_tokens(usage);
    let counts = [
        (UsageMetric::ProviderCacheReadTokens, read),
        (UsageMetric::ProviderCacheCreationTokens, creation),
    ];
    for (metric, count) in counts {
        if count > 0 {
            counters.inc(
                metric,
                labels,
                MetricValue::Finite(Decimal::from(count)),
                now,
            )?;
        }
    }
    Ok(())
}
