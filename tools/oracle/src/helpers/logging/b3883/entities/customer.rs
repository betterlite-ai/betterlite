use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_customer_budget_metrics_after_api_request as set_customer_budget,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::lookups::ScriptedLookups;
use super::super::probe::ProbeLog;
use super::super::support::{
    CUSTOMER_GAUGES, SHAPE_ERROR_MESSAGE, customer_context, finish, helper_error,
};

pub(crate) fn customer_value(input: &Input) -> HelperResult {
    let Some(now) = input.now else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(CUSTOMER_GAUGES, &input.labels_by_metric, input.noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let end_user = EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: input.disable_end_user_cost_tracking,
        enable_prometheus_only: input.enable_prometheus_only,
    };
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let context = customer_context(&settings, end_user, now, &tracker, &limits);
    let scripted = ScriptedLookups::new(input.lookup.clone(), true);
    let request = CustomerBudgetRequest {
        end_user_id: input.end_user_id.as_deref(),
        response_cost: input.response_cost,
    };
    finish(
        &probe,
        &scripted,
        set_customer_budget(&scripted, &gauges, context, end_user, &request),
    )
}
