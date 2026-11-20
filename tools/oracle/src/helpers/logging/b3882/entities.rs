use betterlite_logging::prometheus::budget_gauges::customer::set_customer_budget_metrics;
use betterlite_logging::prometheus::budget_gauges::inputs::{
    CustomerBudgetGaugeInput, KeyBudgetGaugeInput, OrgBudgetGaugeInput, TeamBudgetGaugeInput,
    UserBudgetGaugeInput,
};
use betterlite_logging::prometheus::budget_gauges::key::set_key_budget_metrics;
use betterlite_logging::prometheus::budget_gauges::org::set_org_budget_metrics;
use betterlite_logging::prometheus::budget_gauges::team::set_team_budget_metrics;
use betterlite_logging::prometheus::budget_gauges::user::set_user_budget_metrics;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::Input;
use super::output::samples_value;
use super::probe::{new_log, samples};
use super::support::{
    CUSTOMER_HOURS, CUSTOMER_MAX, CUSTOMER_REMAINING, KEY_HOURS, KEY_MAX, KEY_REMAINING, ORG_HOURS,
    ORG_MAX, ORG_REMAINING, TEAM_HOURS, TEAM_MAX, TEAM_REMAINING, USER_HOURS, USER_MAX,
    USER_REMAINING, customer_context, entity_gauges, label_source, money, runtime_error,
    shape_error,
};

pub(super) fn team_value(input: &Input) -> HelperResult {
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
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(TEAM_REMAINING, TEAM_MAX, TEAM_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = TeamBudgetGaugeInput {
        team_id: team_id.clone(),
        team_alias: team_alias.clone(),
        spend: money(*spend)?,
        max_budget: money(*max_budget)?,
        budget_reset_at: *budget_reset_at,
    };
    set_team_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

pub(super) fn org_value(input: &Input) -> HelperResult {
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
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(ORG_REMAINING, ORG_MAX, ORG_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = OrgBudgetGaugeInput {
        org_id: org_id.clone(),
        org_alias: org_alias.clone().unwrap_or_default(),
        spend: money(*spend)?,
        max_budget: money(*max_budget)?,
        budget_reset_at: *budget_reset_at,
    };
    set_org_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

pub(super) fn key_value(input: &Input) -> HelperResult {
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
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(KEY_REMAINING, KEY_MAX, KEY_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = KeyBudgetGaugeInput {
        token: token.clone(),
        key_alias: key_alias.clone(),
        spend: money(*spend)?,
        max_budget: money(*max_budget)?,
        budget_reset_at: *budget_reset_at,
    };
    set_key_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

pub(super) fn user_value(input: &Input) -> HelperResult {
    let Input::User {
        user_id,
        user_email,
        user_alias,
        spend,
        max_budget,
        budget_reset_at,
        now,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(USER_REMAINING, USER_MAX, USER_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = UserBudgetGaugeInput {
        user_id: user_id.clone(),
        user_email: user_email.clone(),
        user_alias: user_alias.clone(),
        spend: money(*spend)?,
        max_budget: money(*max_budget)?,
        budget_reset_at: *budget_reset_at,
    };
    set_user_budget_metrics(&gauges, &call, label_source(&settings, false), *now)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

pub(super) fn customer_value(input: &Input) -> HelperResult {
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
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(
        CUSTOMER_REMAINING,
        CUSTOMER_MAX,
        CUSTOMER_HOURS,
        labels_by_metric,
        &log,
    )?;
    let settings = PrometheusLabelSettings::with_defaults();
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let call = CustomerBudgetGaugeInput {
        end_user_id: end_user_id.clone(),
        spend: money(*spend)?,
        max_budget: money(*max_budget)?,
        budget_reset_at: *budget_reset_at,
    };
    let context = customer_context(
        &settings,
        *enable_end_user_tracking,
        *now,
        &tracker,
        &limits,
    );
    set_customer_budget_metrics(&gauges, &call, context).map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}
