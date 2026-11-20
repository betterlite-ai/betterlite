use async_trait::async_trait;
use betterlite_logging::alert_settings::cache_keys::SlackAlertingCacheKeys;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

const TOP_DEPLOYMENTS: usize = 5;

// The internal usage DualCache's operations, reduced to the ones the daily report calls.
#[async_trait]
pub trait DailyReportCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<Json>;

    // no ttl: LiteLLM's async_set_cache call for the report stamp passes none, so the cache default applies
    async fn set(&self, key: &str, value: &Json);

    async fn increment(&self, key: &str, amount: f64);

    // none is the outer failure of async_batch_get_cache; the inner option is a key the cache does not hold
    async fn batch_get(&self, keys: &[String]) -> Option<Vec<Option<Json>>>;

    async fn set_many(&self, entries: &[(String, Json)]);
}

// DeploymentMetrics: the record async_update_daily_reports takes.
pub struct DeploymentMetrics<'metrics> {
    pub id: &'metrics str,
    pub failed_request: bool,
    pub latency_per_output_token: Option<f64>,
    pub updated_at: DateTime<Utc>,
}

// The failed keys, the latency keys, and the one batch read that asks for both.
pub struct MetricKeys {
    pub failed: Vec<String>,
    pub latency: Vec<String>,
    pub combined: Vec<String>,
}

#[must_use]
pub fn metric_keys(ids: &[String]) -> MetricKeys {
    let failed: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                "{id}:{}",
                SlackAlertingCacheKeys::FailedRequestsDailyMetrics
            )
        })
        .collect();
    let latency: Vec<String> = ids
        .iter()
        .map(|id| format!("{id}:{}", SlackAlertingCacheKeys::LatencyDailyMetrics))
        .collect();
    let mut combined = failed.clone();
    combined.extend(latency.iter().cloned());
    MetricKeys {
        failed,
        latency,
        combined,
    }
}

// isinstance(val, (int, float)): a JSON number is a metric, anything else counts as no metric at all.
#[must_use]
pub fn metric_number(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) => number.as_f64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// The indices of the top five values, highest first and ties in index order, with zero and negative values left out.
#[must_use]
pub fn top_five(values: &[Option<f64>]) -> Vec<usize> {
    let mut ranked: Vec<(usize, f64)> = values
        .iter()
        .map(|value| value.unwrap_or_default())
        .enumerate()
        .collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    ranked
        .into_iter()
        .filter(|(_, value)| *value > 0.0_f64)
        .take(TOP_DEPLOYMENTS)
        .map(|(index, _)| index)
        .collect()
}

// async_update_daily_reports: increments what the record carries and counts the metrics it set.
pub async fn update_daily_reports<Cache>(cache: &Cache, metrics: DeploymentMetrics<'_>) -> u8
where
    Cache: DailyReportCache,
{
    let mut updated = 0_u8;
    if metrics.failed_request {
        let key = format!(
            "{}:{}",
            metrics.id,
            SlackAlertingCacheKeys::FailedRequestsDailyMetrics
        );
        cache.increment(&key, 1.0_f64).await;
        updated = updated.saturating_add(1);
    }
    if let Some(latency) = metrics.latency_per_output_token {
        let key = format!(
            "{}:{}",
            metrics.id,
            SlackAlertingCacheKeys::LatencyDailyMetrics
        );
        cache.increment(&key, latency).await;
        updated = updated.saturating_add(1);
    }
    updated
}

mod tests;
