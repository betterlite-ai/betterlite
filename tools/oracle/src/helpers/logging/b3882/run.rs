use betterlite_logging::prometheus::budget_gauges::lists::customer_row::set_customer_row_gauges;
use betterlite_logging::prometheus::budget_gauges::remaining_budget::remaining_budget;
use betterlite_logging::prometheus::budget_gauges::reset_hours::hours_until_reset;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;

use super::entities::customer::customer_value;
use super::entities::key::key_value;
use super::entities::org::org_value;
use super::entities::team::team_value;
use super::entities::user::user_value;
use super::gauges::entity_gauges;
use super::input::{BudgetRowCall, Input};
use super::lists::key_list::key_list_value;
use super::lists::org_list::org_list_value;
use super::output::{metric_text, samples_value};
use super::probe::ProbeLog;
use super::provider::provider_value;
use super::support::{
    CUSTOMER_HOURS, CUSTOMER_MAX, CUSTOMER_REMAINING, SHAPE_ERROR_MESSAGE, customer_context,
    helper_error,
};
use super::team_members::team_members_value;

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::RemainingBudget { .. } => remaining_budget_value(input),
        Input::RemainingHours { .. } => remaining_hours_value(input),
        Input::Team { .. } => team_value(input),
        Input::Org { .. } => org_value(input),
        Input::Key { .. } => key_value(input),
        Input::User { .. } => user_value(input),
        Input::Customer { .. } => customer_value(input),
        Input::TeamMembers { .. } => team_members_value(input),
        Input::Provider { .. } => provider_value(input),
        Input::KeyList { .. } => key_list_value(input),
        Input::OrgList { .. } => org_list_value(input),
        Input::CustomerRow { .. } => customer_row_value(input),
    }
}

fn customer_row_value(input: &Input) -> HelperResult {
    let Input::CustomerRow {
        customer,
        default_budget,
        now,
        enable_end_user_tracking,
        labels_by_metric,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauges = entity_gauges(
        CUSTOMER_REMAINING,
        CUSTOMER_MAX,
        CUSTOMER_HOURS,
        labels_by_metric,
        &log,
    )?;
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let row = customer.row();
    let fallback = default_budget.as_ref().map(BudgetRowCall::snapshot);
    let context = customer_context(
        &settings,
        *enable_end_user_tracking,
        *now,
        &tracker,
        &limits,
    );
    set_customer_row_gauges(&gauges, &row, fallback.as_ref(), context)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}

fn remaining_budget_value(input: &Input) -> HelperResult {
    let Input::RemainingBudget { max_budget, spend } = input else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let remaining = remaining_budget(*max_budget, *spend);
    Ok(json!({ "remaining": metric_text(remaining.metric_value()) }))
}

fn remaining_hours_value(input: &Input) -> HelperResult {
    let Input::RemainingHours { reset_at, now } = input else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let hours = hours_until_reset(*reset_at, *now);
    Ok(json!({ "hours": format!("{hours:.6}") }))
}
