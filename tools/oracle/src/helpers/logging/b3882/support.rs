use std::collections::BTreeMap;
use std::time::Instant;

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_gauges::lists::customer_row::CustomerBudgetGaugeRow;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, Utc};

use super::input::{BudgetRowCall, CustomerRowCall};
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
pub(super) const SHAPE_ERROR_MESSAGE: &str = "the call shape does not match its function";

impl BudgetRowCall {
    pub(super) fn snapshot(&self) -> BudgetTableSnapshot {
        BudgetTableSnapshot {
            max_budget: self.max_budget,
            budget_reset_at: self.budget_reset_at,
        }
    }
}

impl CustomerRowCall {
    pub(super) fn row(&self) -> CustomerBudgetGaugeRow {
        CustomerBudgetGaugeRow {
            user_id: self.user_id.clone(),
            spend: self.spend,
            budget: self.budget.as_ref().map(BudgetRowCall::snapshot),
        }
    }
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
        collector: ProbeGauge {
            metric,
            labels: Vec::new(),
            log: log.clone(),
        },
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
