use betterlite_logging::prometheus::budget_lookups::snapshots::UserBudgetSnapshot;
use betterlite_logging::prometheus::budget_lookups::user_budget::{
    UserBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_user_budget_metrics_after_api_request as set_user_budget,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::lookups::ScriptedLookups;
use super::super::probe::ProbeLog;
use super::super::support::{SHAPE_ERROR_MESSAGE, USER_GAUGES, finish, helper_error, label_source};

pub(crate) fn user_value(input: &Input) -> HelperResult {
    let Some(now) = input.now else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(USER_GAUGES, &input.labels_by_metric, input.noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(input.lookup.clone(), true);
    let carried = input.carried.as_ref().map(|call| UserBudgetSnapshot {
        budget_reset_at: call.budget_reset_at,
        max_budget: call.max_budget,
        user_alias: call.user_alias.clone(),
    });
    let request = UserBudgetRequest {
        user_id: input.user_id.as_deref(),
        user_spend: input.user_spend,
        user_max_budget: input.user_max_budget,
        response_cost: input.response_cost,
        carried: carried.as_ref(),
        user_email: input.user_email.as_deref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_user_budget(&scripted, &gauges, source, now, &request),
    )
}
