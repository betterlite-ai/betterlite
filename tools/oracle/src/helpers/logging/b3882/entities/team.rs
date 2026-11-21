use betterlite_logging::prometheus::budget_gauges::inputs::TeamBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::team::set_team_budget_metrics;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::output::samples_value;
use super::super::probe::ProbeLog;
use super::super::support::{
    SHAPE_ERROR_MESSAGE, TEAM_HOURS, TEAM_MAX, TEAM_REMAINING, helper_error, label_source,
};

pub(crate) fn team_value(input: &Input) -> HelperResult {
    let Input::Team {
        team_id,
        team_alias,
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
    let gauges = entity_gauges(TEAM_REMAINING, TEAM_MAX, TEAM_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = TeamBudgetGaugeInput {
        team_id: team_id.clone(),
        team_alias: team_alias.clone(),
        spend: *spend,
        max_budget: *max_budget,
        budget_reset_at: *budget_reset_at,
    };
    set_team_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
