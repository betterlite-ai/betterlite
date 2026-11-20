use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;

use super::BudgetGaugeError;
use super::customer::{CustomerGaugeContext, set_customer_budget_metrics};
use super::gauge::EntityBudgetGauges;
use super::inputs::{
    CustomerBudgetGaugeInput, KeyBudgetGaugeInput, OrgBudgetGaugeInput, TeamBudgetGaugeInput,
    UserBudgetGaugeInput,
};
use super::key::set_key_budget_metrics;
use super::org::set_org_budget_metrics;
use super::team::set_team_budget_metrics;
use super::user::set_user_budget_metrics;

// LiteLLM's refresh reads a list of key strings or full key records and sets metrics only for the full records
pub enum KeyListEntry {
    Record(KeyBudgetGaugeInput),
    Token(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetTableSnapshot {
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgBudgetGaugeRow {
    pub organization_id: Option<String>,
    pub organization_alias: Option<String>,
    pub spend: Option<Decimal>,
    pub budget: Option<BudgetTableSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomerBudgetGaugeRow {
    pub user_id: String,
    pub spend: Decimal,
    pub budget: Option<BudgetTableSnapshot>,
}

pub fn set_key_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    entries: &[KeyListEntry],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for entry in entries {
        if let KeyListEntry::Record(input) = entry {
            set_key_budget_metrics(gauges, input, source, now)?;
        }
    }
    Ok(())
}

pub fn set_team_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    teams: &[TeamBudgetGaugeInput],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for team in teams {
        set_team_budget_metrics(gauges, team, source, now)?;
    }
    Ok(())
}

pub fn set_user_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    users: &[UserBudgetGaugeInput],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for user in users {
        set_user_budget_metrics(gauges, user, source, now)?;
    }
    Ok(())
}

pub fn set_org_list_budget_metrics<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    orgs: &[OrgBudgetGaugeRow],
    source: LabelSource<'_>,
    now: DateTime<Utc>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    for org in orgs {
        let budget = org.budget.as_ref();
        let input = OrgBudgetGaugeInput {
            org_id: org.organization_id.clone().unwrap_or_default(),
            org_alias: org.organization_alias.clone().unwrap_or_default(),
            spend: Some(org.spend.unwrap_or_default()),
            max_budget: budget.and_then(|table| table.max_budget),
            budget_reset_at: budget.and_then(|table| table.budget_reset_at),
        };
        set_org_budget_metrics(gauges, &input, source, now)?;
    }
    Ok(())
}

// the one customer path that passes a reset time: the refresh reads it from the row's budget table
// ast-grep-ignore: no-long-names
pub fn set_customer_budget_metrics_from_row<Gauge>(
    gauges: &EntityBudgetGauges<Gauge>,
    customer: &CustomerBudgetGaugeRow,
    default_budget: Option<&BudgetTableSnapshot>,
    context: CustomerGaugeContext<'_>,
) -> Result<(), BudgetGaugeError<Gauge::Error>>
where
    Gauge: Collector + RemovableMetric,
{
    let budget = customer.budget.as_ref().or(default_budget);
    let input = CustomerBudgetGaugeInput {
        end_user_id: customer.user_id.clone(),
        spend: Some(customer.spend),
        max_budget: budget.and_then(|table| table.max_budget),
        budget_reset_at: budget.and_then(|table| table.budget_reset_at),
    };
    set_customer_budget_metrics(gauges, &input, context)
}

mod tests;
