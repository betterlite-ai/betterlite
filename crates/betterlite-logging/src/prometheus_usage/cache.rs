use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use super::counters::{UsageCounters, UsageLabels};
use super::error::PrometheusUsageError;
use super::metric::UsageMetric;
use super::usage_object::{object, resolve_provider_cache_tokens};
use crate::fields::payload::StandardLoggingPayload;

pub fn increment_cache_metrics<C: UsageCounters>(
    counters: &C,
    payload: &StandardLoggingPayload,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<C::Counter::Error>> {
    match payload.cache_hit {
        Some(true) => {
            counters.inc(
                UsageMetric::CacheHits,
                labels,
                MetricValue::Finite(Decimal::ONE),
                now,
            )?;
            if payload.total_tokens > 0 {
                counters.inc(
                    UsageMetric::CachedTokens,
                    labels,
                    MetricValue::Finite(Decimal::from(payload.total_tokens)),
                    now,
                )?;
            }
        }
        Some(false) => {
            counters.inc(
                UsageMetric::CacheMisses,
                labels,
                MetricValue::Finite(Decimal::ONE),
                now,
            )?;
        }
        None => {}
    }
    let Some(usage) = object(payload.metadata.usage_object.as_ref()) else {
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
