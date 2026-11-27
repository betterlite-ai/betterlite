use betterlite_logging::prometheus::budget_lookups::snapshots::TeamBudgetSnapshot;
use betterlite_logging::prometheus::budget_lookups::team_budget::{
    TeamBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_team_budget_metrics_after_api_request as set_team_budget,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::lookups::ScriptedLookups;
use super::super::probe::ProbeLog;
use super::super::support::{SHAPE_ERROR_MESSAGE, TEAM_GAUGES, finish, helper_error, label_source};

pub(crate) fn team_value(input: &Input) -> HelperResult {
    let Some(now) = input.now else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(TEAM_GAUGES, &input.labels_by_metric, input.noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(input.lookup.clone(), true);
    let carried = input.carried.as_ref().map(|call| TeamBudgetSnapshot {
        budget_reset_at: call.budget_reset_at,
        max_budget: call.max_budget,
    });
    let request = TeamBudgetRequest {
        user_api_team: input.team_id.as_deref(),
        user_api_team_alias: input.team_alias.as_deref(),
        team_spend: input.team_spend,
        team_max_budget: input.team_max_budget,
        response_cost: input.response_cost,
        carried: carried.as_ref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_team_budget(&scripted, &gauges, source, now, &request),
    )
}
