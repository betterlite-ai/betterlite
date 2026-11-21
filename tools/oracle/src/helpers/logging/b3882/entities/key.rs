use betterlite_logging::prometheus::budget_gauges::inputs::KeyBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::key::set_key_budget_metrics;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::output::samples_value;
use super::super::probe::ProbeLog;
use super::super::support::{
    KEY_HOURS, KEY_MAX, KEY_REMAINING, SHAPE_ERROR_MESSAGE, helper_error, label_source,
};

pub(crate) fn key_value(input: &Input) -> HelperResult {
    let Input::Key {
        token,
        key_alias,
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
    let gauges = entity_gauges(KEY_REMAINING, KEY_MAX, KEY_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = KeyBudgetGaugeInput {
        token: token.clone(),
        key_alias: key_alias.clone(),
        spend: *spend,
        max_budget: *max_budget,
        budget_reset_at: *budget_reset_at,
    };
    set_key_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
