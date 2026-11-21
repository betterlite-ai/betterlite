use betterlite_logging::prometheus::budget_gauges::inputs::UserBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::user::set_user_budget_metrics;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::output::samples_value;
use super::super::probe::ProbeLog;
use super::super::support::{
    SHAPE_ERROR_MESSAGE, USER_HOURS, USER_MAX, USER_REMAINING, helper_error, label_source,
};

pub(crate) fn user_value(input: &Input) -> HelperResult {
    let Input::User {
        user_id,
        user_email,
        user_alias,
        spend,
        max_budget,
        budget_reset_at,
        now,
        labels_by_metric,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauges = entity_gauges(USER_REMAINING, USER_MAX, USER_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = UserBudgetGaugeInput {
        user_id: user_id.clone(),
        user_email: user_email.clone(),
        user_alias: user_alias.clone(),
        spend: *spend,
        max_budget: *max_budget,
        budget_reset_at: *budget_reset_at,
    };
    set_user_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
