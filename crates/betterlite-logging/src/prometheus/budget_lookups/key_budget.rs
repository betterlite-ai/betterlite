use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus::budget_gauges::BudgetGaugeError;
use crate::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use crate::prometheus::budget_gauges::inputs::KeyBudgetGaugeInput;
use crate::prometheus::budget_gauges::key::set_key_budget_metrics;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::lookups::BudgetLookups;
use super::snapshots::KeyBudgetSnapshot;

pub struct KeyBudgetRequest<'request> {
    pub user_api_key: Option<&'request str>,
    pub user_api_key_alias: Option<&'request str>,
    pub response_cost: Decimal,
    pub key_max_budget: Option<Decimal>,
    pub key_spend: Option<Decimal>,
    pub carried: Option<&'request KeyBudgetSnapshot>,
}

// LiteLLM's _set_api_key_budget_metrics_after_api_request: the no-op gauge and the key come first, then the assembly
pub async fn set_key_budget_metrics_after_api_request<Gauge>(
    lookups: &dyn BudgetLookups,
    gauges: &EntityBudgetGauges<MetricLike<Gauge>>,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
    request: &KeyBudgetRequest<'_>,
) -> Result<(), BudgetGaugeError<<MetricLike<Gauge> as RemovableMetric>::Error>>
where
    MetricLike<Gauge>: Collector + RemovableMetric + Sync,
{
    if matches!(&gauges.remaining.collector, MetricLike::NoOp(_)) {
        return Ok(());
    }
    let Some(user_api_key) = request.user_api_key else {
        return Ok(());
    };
    let key = assemble_key_object(lookups, user_api_key, request).await;
    set_key_budget_metrics(gauges, &key, source, now)
}

// LiteLLM's _assemble_key_object: the fallback is a cache-only read, so a cache miss leaves the reset time unset
async fn assemble_key_object(
    lookups: &dyn BudgetLookups,
    user_api_key: &str,
    request: &KeyBudgetRequest<'_>,
) -> KeyBudgetGaugeInput {
    let spend = request
        .key_spend
        .unwrap_or(Decimal::ZERO)
        .saturating_add(request.response_cost);
    let key_alias = request.user_api_key_alias.map(str::to_owned);
    if let Some(carried) = request.carried {
        return KeyBudgetGaugeInput {
            token: user_api_key.to_owned(),
            key_alias,
            spend: Some(spend),
            max_budget: request.key_max_budget,
            budget_reset_at: carried.budget_reset_at,
        };
    }
    let mut key = KeyBudgetGaugeInput {
        token: user_api_key.to_owned(),
        key_alias,
        spend: Some(spend),
        max_budget: request.key_max_budget,
        budget_reset_at: None,
    };
    match lookups.key_info(user_api_key, true).await {
        Ok(record) => {
            key.budget_reset_at = record.budget_reset_at;
        }
        Err(error) => {
            tracing::debug!("[Non-Blocking] Prometheus: Error getting key info: {error}");
        }
    }
    key
}

mod tests;
