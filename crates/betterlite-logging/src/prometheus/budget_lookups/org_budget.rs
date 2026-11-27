use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus::budget_gauges::BudgetGaugeError;
use crate::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use crate::prometheus::budget_gauges::inputs::OrgBudgetGaugeInput;
use crate::prometheus::budget_gauges::org::set_org_budget_metrics;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::lookups::BudgetLookups;
use super::records::OrgRecord;
use super::snapshots::OrgBudgetSnapshot;

pub struct OrgBudgetRequest<'request> {
    pub org_id: Option<&'request str>,
    pub response_cost: Decimal,
    pub carried: Option<&'request OrgBudgetSnapshot>,
    pub org_alias: Option<&'request str>,
}

// LiteLLM's _set_org_budget_metrics_after_api_request: a carried snapshot skips the database, and a missing
// database client or org id skips everything
pub async fn set_org_budget_metrics_after_api_request<Gauge>(
    lookups: &dyn BudgetLookups,
    gauges: &EntityBudgetGauges<MetricLike<Gauge>>,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
    request: &OrgBudgetRequest<'_>,
) -> Result<(), BudgetGaugeError<<MetricLike<Gauge> as RemovableMetric>::Error>>
where
    MetricLike<Gauge>: Collector + RemovableMetric + Sync,
{
    if matches!(&gauges.remaining.collector, MetricLike::NoOp(_)) {
        return Ok(());
    }
    let Some(org_id) = request.org_id else {
        return Ok(());
    };
    if let Some(carried) = request.carried {
        let input = carried_input(org_id, request, carried);
        return set_org_budget_metrics(gauges, &input, source, now);
    }
    if !lookups.database_configured() {
        return Ok(());
    }
    match lookups.org_info(org_id, true).await {
        Ok(Some(record)) => {
            let input = record_input(org_id, request, record);
            set_org_budget_metrics(gauges, &input, source, now)
        }
        Ok(None) => Ok(()),
        Err(error) => {
            tracing::debug!("[Non-Blocking] Prometheus: Error getting org info: {error}");
            Ok(())
        }
    }
}

// the carried path reports no reset time, unlike every other entity
fn carried_input(
    org_id: &str,
    request: &OrgBudgetRequest<'_>,
    carried: &OrgBudgetSnapshot,
) -> OrgBudgetGaugeInput {
    OrgBudgetGaugeInput {
        org_id: org_id.to_owned(),
        org_alias: request.org_alias.unwrap_or_default().to_owned(),
        spend: carried.spend + request.response_cost,
        max_budget: carried.max_budget,
        budget_reset_at: None,
    }
}

// the fetched path reads the alias from the org row and the budget from its joined budget table
fn record_input(
    org_id: &str,
    request: &OrgBudgetRequest<'_>,
    record: OrgRecord,
) -> OrgBudgetGaugeInput {
    let budget_table = record.litellm_budget_table.as_ref();
    OrgBudgetGaugeInput {
        org_id: org_id.to_owned(),
        org_alias: record.organization_alias.unwrap_or_default(),
        spend: record.spend.unwrap_or(Decimal::ZERO) + request.response_cost,
        max_budget: budget_table.and_then(|table| table.max_budget),
        budget_reset_at: budget_table.and_then(|table| table.budget_reset_at),
    }
}

mod tests;
