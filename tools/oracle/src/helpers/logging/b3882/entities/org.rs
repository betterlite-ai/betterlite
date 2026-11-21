use betterlite_logging::prometheus::budget_gauges::inputs::OrgBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::org::set_org_budget_metrics;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::output::samples_value;
use super::super::probe::ProbeLog;
use super::super::support::{
    ORG_HOURS, ORG_MAX, ORG_REMAINING, SHAPE_ERROR_MESSAGE, helper_error, label_source,
};

pub(crate) fn org_value(input: &Input) -> HelperResult {
    let Input::Org {
        org_id,
        org_alias,
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
    let gauges = entity_gauges(ORG_REMAINING, ORG_MAX, ORG_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = OrgBudgetGaugeInput {
        org_id: org_id.clone(),
        org_alias: org_alias.clone().unwrap_or_default(),
        spend: *spend,
        max_budget: *max_budget,
        budget_reset_at: *budget_reset_at,
    };
    set_org_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
