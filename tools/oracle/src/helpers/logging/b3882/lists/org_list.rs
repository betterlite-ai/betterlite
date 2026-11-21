use betterlite_logging::prometheus::budget_gauges::lists::org_list::{
    OrgBudgetGaugeRow, set_org_list_budget_metrics,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::{BudgetRowCall, Input, OrgRowCall};
use super::super::output::samples_value;
use super::super::probe::ProbeLog;
use super::super::support::{
    ORG_HOURS, ORG_MAX, ORG_REMAINING, SHAPE_ERROR_MESSAGE, helper_error, label_source,
};

impl OrgRowCall {
    fn row(&self) -> OrgBudgetGaugeRow {
        OrgBudgetGaugeRow {
            organization_id: self.organization_id.clone(),
            organization_alias: self.organization_alias.clone(),
            spend: self.spend,
            budget: self.budget.as_ref().map(BudgetRowCall::snapshot),
        }
    }
}

pub(crate) fn org_list_value(input: &Input) -> HelperResult {
    let Input::OrgList {
        rows,
        now,
        labels_by_metric,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauges = entity_gauges(ORG_REMAINING, ORG_MAX, ORG_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let calls: Vec<_> = rows.iter().map(OrgRowCall::row).collect();
    set_org_list_budget_metrics(&gauges, &calls, label_source(&settings, false), *now)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
