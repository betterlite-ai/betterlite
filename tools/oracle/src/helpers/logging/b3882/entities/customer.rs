use betterlite_logging::prometheus::budget_gauges::customer::set_customer_budget_metrics;
use betterlite_logging::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use betterlite_logging::prometheus::budget_gauges::inputs::CustomerBudgetGaugeInput;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use chrono::{DateTime, Utc};

use super::super::gauges::entity_gauges;
use super::super::input::Input;
use super::super::output::samples_value;
use super::super::probe::{ProbeGauge, ProbeLog};
use super::super::support::{
    CUSTOMER_HOURS, CUSTOMER_MAX, CUSTOMER_REMAINING, SHAPE_ERROR_MESSAGE, customer_context,
    helper_error,
};

pub(crate) fn customer_value(input: &Input) -> HelperResult {
    let Input::Customer {
        end_user_id,
        spend,
        max_budget,
        budget_reset_at,
        now,
        enable_end_user_tracking,
        labels_by_metric,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauges = entity_gauges(
        CUSTOMER_REMAINING,
        CUSTOMER_MAX,
        CUSTOMER_HOURS,
        labels_by_metric,
        &log,
    )?;
    let call = CustomerBudgetGaugeInput {
        end_user_id: end_user_id.clone(),
        spend: *spend,
        max_budget: *max_budget,
        budget_reset_at: *budget_reset_at,
    };
    write_customer(&gauges, &call, *now, *enable_end_user_tracking)?;
    Ok(samples_value(&log.drain()))
}

fn write_customer(
    gauges: &EntityBudgetGauges<ProbeGauge>,
    call: &CustomerBudgetGaugeInput,
    now: DateTime<Utc>,
    enabled: bool,
) -> Result<(), Box<HelperError>> {
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let context = customer_context(&settings, enabled, now, &tracker, &limits);
    set_customer_budget_metrics(gauges, call, context)
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))
}
