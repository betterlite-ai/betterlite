use betterlite_logging::prometheus::budget_lookups::key_budget::{
    KeyBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_key_budget_metrics_after_api_request as set_key_budget,
};
use betterlite_logging::prometheus::budget_lookups::snapshots::KeyBudgetSnapshot;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::lookups::ScriptedLookups;
use super::super::probe::ProbeLog;
use super::super::support::{KEY_GAUGES, SHAPE_ERROR_MESSAGE, finish, helper_error, label_source};

pub(crate) fn key_value(input: &Input) -> HelperResult {
    let Some(now) = input.now else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(KEY_GAUGES, &input.labels_by_metric, input.noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(input.lookup.clone(), true);
    let carried = input.carried.as_ref().map(|call| KeyBudgetSnapshot {
        budget_reset_at: call.budget_reset_at,
    });
    let request = KeyBudgetRequest {
        user_api_key: input.user_api_key.as_deref(),
        user_api_key_alias: input.user_api_key_alias.as_deref(),
        response_cost: input.response_cost,
        key_max_budget: input.key_max_budget,
        key_spend: input.key_spend,
        carried: carried.as_ref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_key_budget(&scripted, &gauges, source, now, &request),
    )
}
