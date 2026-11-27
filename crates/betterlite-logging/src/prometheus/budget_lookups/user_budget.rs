use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus::budget_gauges::BudgetGaugeError;
use crate::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use crate::prometheus::budget_gauges::inputs::UserBudgetGaugeInput;
use crate::prometheus::budget_gauges::user::set_user_budget_metrics;
use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::lookups::{BudgetLookups, UserLookupOptions};
use super::records::UserRecord;
use super::snapshots::UserBudgetSnapshot;

pub struct UserBudgetRequest<'request> {
    pub user_id: Option<&'request str>,
    pub user_spend: Option<Decimal>,
    pub user_max_budget: Option<Decimal>,
    pub response_cost: Decimal,
    pub carried: Option<&'request UserBudgetSnapshot>,
    pub user_email: Option<&'request str>,
}

// LiteLLM's _set_user_budget_metrics_after_api_request: the no-op gauge and the user id come first, then the assembly
pub async fn set_user_budget_metrics_after_api_request<Gauge>(
    lookups: &dyn BudgetLookups,
    gauges: &EntityBudgetGauges<MetricLike<Gauge>>,
    source: LabelSource<'_>,
    now: DateTime<Utc>,
    request: &UserBudgetRequest<'_>,
) -> Result<(), BudgetGaugeError<<MetricLike<Gauge> as RemovableMetric>::Error>>
where
    MetricLike<Gauge>: Collector + RemovableMetric + Sync,
{
    if matches!(&gauges.remaining.collector, MetricLike::NoOp(_)) {
        return Ok(());
    }
    let Some(user_id) = request.user_id else {
        return Ok(());
    };
    let user = assemble_user_object(lookups, user_id, request).await;
    set_user_budget_metrics(gauges, &user, source, now)
}

// LiteLLM's _assemble_user_object: cache-first, never db-only, and the request's email only counts on the carried path
async fn assemble_user_object(
    lookups: &dyn BudgetLookups,
    user_id: &str,
    request: &UserBudgetRequest<'_>,
) -> UserBudgetGaugeInput {
    let spend = request.user_spend.unwrap_or(Decimal::ZERO) + request.response_cost;
    if let Some(carried) = request.carried {
        return UserBudgetGaugeInput {
            user_id: user_id.to_owned(),
            user_email: request.user_email.map(str::to_owned),
            user_alias: carried.user_alias.clone(),
            spend,
            max_budget: request.user_max_budget.or(carried.max_budget),
            budget_reset_at: carried.budget_reset_at,
        };
    }
    let mut user = UserBudgetGaugeInput {
        user_id: user_id.to_owned(),
        user_email: None,
        user_alias: None,
        spend,
        max_budget: request.user_max_budget,
        budget_reset_at: None,
    };
    let options = UserLookupOptions {
        user_id_upsert: false,
        check_db_only: false,
    };
    match lookups.user_info(user_id, options).await {
        Ok(Some(record)) => apply_user_record(&mut user, record),
        Ok(None) => {}
        Err(error) => {
            tracing::debug!("[Non-Blocking] Prometheus: Error getting user info: {error}");
        }
    }
    user
}

// a fetched record always replaces the reset time, and fills only the fields the request left unset
fn apply_user_record(user: &mut UserBudgetGaugeInput, record: UserRecord) {
    user.budget_reset_at = record.budget_reset_at;
    if user.max_budget.is_none() && record.max_budget.is_some() {
        user.max_budget = record.max_budget;
    }
    if let Some(email) = record.user_email {
        user.user_email = Some(email);
    }
    if let Some(alias) = record.user_alias {
        user.user_alias = Some(alias);
    }
}

mod tests;
