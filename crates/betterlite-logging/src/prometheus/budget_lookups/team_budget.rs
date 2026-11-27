use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus::budget_gauges::BudgetGaugeError;
use crate::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use crate::prometheus::budget_gauges::inputs::TeamBudgetGaugeInput;
use crate::prometheus::budget_gauges::team::set_team_budget_metrics;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::lookups::BudgetLookups;
use super::snapshots::TeamBudgetSnapshot;

pub struct TeamBudgetRequest<'request> {
    pub user_api_team: Option<&'request str>,
    pub user_api_team_alias: Option<&'request str>,
    pub team_spend: Option<Decimal>,
    pub team_max_budget: Option<Decimal>,
    pub response_cost: Decimal,
    pub carried: Option<&'request TeamBudgetSnapshot>,
}

// LiteLLM's _set_team_budget_metrics_after_api_request: the no-op gauge and the team id come first, then the assembly
pub async fn set_team_budget_metrics_after_api_request<Gauge>(
    lookups: &dyn BudgetLookups,
    gauges: &EntityBudgetGauges<MetricLike<Gauge>>,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
    request: &TeamBudgetRequest<'_>,
) -> Result<(), BudgetGaugeError<<MetricLike<Gauge> as RemovableMetric>::Error>>
where
    MetricLike<Gauge>: Collector + RemovableMetric + Sync,
{
    if matches!(&gauges.remaining.collector, MetricLike::NoOp(_)) {
        return Ok(());
    }
    let Some(team_id) = request.user_api_team else {
        return Ok(());
    };
    let team = assemble_team_object(lookups, team_id, request).await;
    set_team_budget_metrics(gauges, &team, source, now)
}

// LiteLLM's _assemble_team_object: a carried snapshot wins, and a fetched record only fills what the request left unset
async fn assemble_team_object(
    lookups: &dyn BudgetLookups,
    team_id: &str,
    request: &TeamBudgetRequest<'_>,
) -> TeamBudgetGaugeInput {
    let spend = request
        .team_spend
        .unwrap_or(Decimal::ZERO)
        .saturating_add(request.response_cost);
    let team_alias = request.user_api_team_alias.map(str::to_owned);
    if let Some(carried) = request.carried {
        return TeamBudgetGaugeInput {
            team_id: team_id.to_owned(),
            team_alias,
            spend: Some(spend),
            max_budget: request.team_max_budget.or(carried.max_budget),
            budget_reset_at: carried.budget_reset_at,
        };
    }
    let mut team = TeamBudgetGaugeInput {
        team_id: team_id.to_owned(),
        team_alias,
        spend: Some(spend),
        max_budget: request.team_max_budget,
        budget_reset_at: None,
    };
    match lookups.team_info(team_id).await {
        Ok(record) => {
            team.budget_reset_at = record.budget_reset_at;
            if team.max_budget.is_none() && record.max_budget.is_some() {
                team.max_budget = record.max_budget;
            }
        }
        Err(error) => {
            tracing::debug!("[Non-Blocking] Prometheus: Error getting team info: {error}");
        }
    }
    team
}

mod tests;
