use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use indexmap::IndexMap;
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
    if let Some(usage) = object(payload.metadata.usage_object.as_ref()) {
        increment_provider_cache(counters, usage, labels, now)?;
    }
    Ok(())
}

fn increment_provider_cache<C: UsageCounters>(
    counters: &C,
    usage: &IndexMap<String, Json>,
    labels: UsageLabels<'_>,
    now: Instant,
) -> Result<(), PrometheusUsageError<C::Counter::Error>> {
    let (read, creation) = resolve_provider_cache_tokens(usage);
    if read > 0 {
        counters.inc(
            UsageMetric::ProviderCacheReadTokens,
            labels,
            MetricValue::Finite(Decimal::from(read)),
            now,
        )?;
    }
    if creation > 0 {
        counters.inc(
            UsageMetric::ProviderCacheCreationTokens,
            labels,
            MetricValue::Finite(Decimal::from(creation)),
            now,
        )?;
    }
    Ok(())
}
