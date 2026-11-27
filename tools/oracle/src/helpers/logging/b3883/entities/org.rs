use betterlite_logging::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_org_budget_metrics_after_api_request as set_org_budget,
};
use betterlite_logging::prometheus::budget_lookups::snapshots::OrgBudgetSnapshot;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;
use rust_decimal::Decimal;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::lookups::ScriptedLookups;
use super::super::probe::ProbeLog;
use super::super::support::{ORG_GAUGES, SHAPE_ERROR_MESSAGE, finish, helper_error, label_source};

pub(crate) fn org_value(input: &Input) -> HelperResult {
    let Some(now) = input.now else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(ORG_GAUGES, &input.labels_by_metric, input.noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(input.lookup.clone(), input.database);
    let carried = input.carried.as_ref().map(|call| OrgBudgetSnapshot {
        spend: call.spend.unwrap_or(Decimal::ZERO),
        max_budget: call.max_budget,
    });
    let request = OrgBudgetRequest {
        org_id: input.org_id.as_deref(),
        response_cost: input.response_cost,
        carried: carried.as_ref(),
        org_alias: input.org_alias.as_deref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_org_budget(&scripted, &gauges, source, now, &request),
    )
}
