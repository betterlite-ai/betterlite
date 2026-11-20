use std::collections::BTreeMap;
use std::error::Error;
use std::time::Instant;

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::BudgetGaugeError;
use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_gauges::gauge::{EntityBudgetGauges, GaugeField};
use betterlite_logging::prometheus::budget_gauges::inputs::KeyBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::lists::{
    BudgetTableSnapshot, CustomerBudgetGaugeRow, KeyListEntry, OrgBudgetGaugeRow,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::input::{BudgetRowCall, CustomerRowCall, KeyListEntryCall, OrgRowCall};
use super::probe::{ProbeGauge, ProbeLog};

pub(super) const TEAM_REMAINING: &str = "litellm_remaining_team_budget_metric";
pub(super) const TEAM_MAX: &str = "litellm_team_max_budget_metric";
pub(super) const TEAM_HOURS: &str = "litellm_team_budget_remaining_hours_metric";
pub(super) const TEAM_MEMBERS: &str = "litellm_team_members_metric";
pub(super) const ORG_REMAINING: &str = "litellm_remaining_org_budget_metric";
pub(super) const ORG_MAX: &str = "litellm_org_max_budget_metric";
pub(super) const ORG_HOURS: &str = "litellm_org_budget_remaining_hours_metric";
pub(super) const KEY_REMAINING: &str = "litellm_remaining_api_key_budget_metric";
pub(super) const KEY_MAX: &str = "litellm_api_key_max_budget_metric";
pub(super) const KEY_HOURS: &str = "litellm_api_key_budget_remaining_hours_metric";
pub(super) const USER_REMAINING: &str = "litellm_remaining_user_budget_metric";
pub(super) const USER_MAX: &str = "litellm_user_max_budget_metric";
pub(super) const USER_HOURS: &str = "litellm_user_budget_remaining_hours_metric";
pub(super) const CUSTOMER_REMAINING: &str = "litellm_remaining_customer_budget_metric";
pub(super) const CUSTOMER_MAX: &str = "litellm_customer_max_budget_metric";
pub(super) const CUSTOMER_HOURS: &str = "litellm_customer_budget_remaining_hours_metric";
pub(super) const PROVIDER_REMAINING: &str = "litellm_provider_remaining_budget_metric";

pub(super) fn key_list_entry(call: &KeyListEntryCall) -> Result<KeyListEntry, Box<HelperError>> {
    match call {
        KeyListEntryCall::Record {
            token,
            key_alias,
            spend,
            max_budget,
            budget_reset_at,
        } => Ok(KeyListEntry::Record(KeyBudgetGaugeInput {
            token: token.clone(),
            key_alias: key_alias.clone(),
            spend: money(*spend)?,
            max_budget: money(*max_budget)?,
            budget_reset_at: *budget_reset_at,
        })),
        KeyListEntryCall::Token { token } => Ok(KeyListEntry::Token(token.clone())),
    }
}

pub(super) fn org_row(call: &OrgRowCall) -> Result<OrgBudgetGaugeRow, Box<HelperError>> {
    Ok(OrgBudgetGaugeRow {
        organization_id: call.organization_id.clone(),
        organization_alias: call.organization_alias.clone(),
        spend: money(call.spend)?,
        budget: budget_row(call.budget.as_ref())?,
    })
}

pub(super) fn customer_row(
    call: &CustomerRowCall,
) -> Result<CustomerBudgetGaugeRow, Box<HelperError>> {
    Ok(CustomerBudgetGaugeRow {
        user_id: call.user_id.clone(),
        spend: required_money(call.spend)?,
        budget: budget_row(call.budget.as_ref())?,
    })
}

pub(super) fn budget_row(
    call: Option<&BudgetRowCall>,
) -> Result<Option<BudgetTableSnapshot>, Box<HelperError>> {
    match call {
        Some(table) => Ok(Some(BudgetTableSnapshot {
            max_budget: money(table.max_budget)?,
            budget_reset_at: table.budget_reset_at,
        })),
        None => Ok(None),
    }
}

pub(super) fn money(value: Option<f64>) -> Result<Option<Decimal>, Box<HelperError>> {
    value
        .map(|amount| {
            Decimal::from_f64(amount)
                .ok_or_else(|| helper_error("InvalidInput", "a money value is not finite"))
        })
        .transpose()
}

pub(super) fn required_money(value: f64) -> Result<Decimal, Box<HelperError>> {
    let amount = Decimal::from_f64(value)
        .ok_or_else(|| helper_error("InvalidInput", "a money value is not finite"))?;
    Ok(amount)
}

pub(super) fn field(
    metric: &'static str,
    labels: &BTreeMap<String, Vec<String>>,
    log: &ProbeLog,
) -> Result<GaugeField<ProbeGauge>, Box<HelperError>> {
    let supported_labels = labels.get(metric).cloned().ok_or_else(|| {
        helper_error("InvalidInput", "the call has no label list for this metric")
    })?;
    Ok(GaugeField {
        metric_name: metric,
        supported_labels,
        collector: ProbeGauge::new(metric, log),
    })
}

pub(super) fn entity_gauges(
    remaining: &'static str,
    max: &'static str,
    hours: &'static str,
    labels: &BTreeMap<String, Vec<String>>,
    log: &ProbeLog,
) -> Result<EntityBudgetGauges<ProbeGauge>, Box<HelperError>> {
    Ok(EntityBudgetGauges {
        remaining: field(remaining, labels, log)?,
        max: field(max, labels, log)?,
        remaining_hours: field(hours, labels, log)?,
    })
}

pub(super) fn label_source(
    settings: &PrometheusLabelSettings,
    track_end_user: bool,
) -> LabelSource<'_> {
    LabelSource::Values {
        settings,
        end_user_settings: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: None,
            enable_prometheus_only: Some(track_end_user),
        },
    }
}

pub(super) fn customer_context<'borrow>(
    settings: &'borrow PrometheusLabelSettings,
    track_end_user: bool,
    now: DateTime<Utc>,
    tracker: &'borrow BoundedPrometheusSeriesTracker,
    limits: &'borrow SeriesLimits,
) -> CustomerGaugeContext<'borrow> {
    CustomerGaugeContext {
        source: label_source(settings, track_end_user),
        clock: BudgetGaugeClock {
            reset: now,
            monotonic: Instant::now(),
        },
        tracker,
        limits,
    }
}

pub(super) fn runtime_error<MetricError>(error: BudgetGaugeError<MetricError>) -> Box<HelperError>
where
    MetricError: Error + 'static,
{
    helper_error("RuntimeError", &error.to_string())
}

pub(super) fn shape_error() -> Box<HelperError> {
    helper_error("InvalidInput", "the call shape does not match its function")
}

pub(super) fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
