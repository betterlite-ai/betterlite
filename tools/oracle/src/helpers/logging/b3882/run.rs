use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus::budget_gauges::inputs::TeamMembersGaugeInput;
use betterlite_logging::prometheus::budget_gauges::lists::{
    KeyListEntry, OrgBudgetGaugeRow, set_customer_budget_metrics_from_row,
    set_key_list_budget_metrics, set_org_list_budget_metrics,
};
use betterlite_logging::prometheus::budget_gauges::remaining_budget::remaining_budget;
use betterlite_logging::prometheus::budget_gauges::reset_hours::remaining_hours_for_budget_reset;
use betterlite_logging::prometheus::budget_gauges::small_gauges::{
    set_team_members_metric, track_provider_remaining_budget,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use serde_json::json;

use super::entities::{customer_value, key_value, org_value, team_value, user_value};
use super::input::Input;
use super::output::{metric_text, samples_value};
use super::probe::{ProbeGauge, new_log, samples};
use super::support::{
    CUSTOMER_HOURS, CUSTOMER_MAX, CUSTOMER_REMAINING, KEY_HOURS, KEY_MAX, KEY_REMAINING, ORG_HOURS,
    ORG_MAX, ORG_REMAINING, PROVIDER_REMAINING, TEAM_MEMBERS, budget_row, customer_context,
    customer_row, entity_gauges, field, helper_error, key_list_entry, label_source, money, org_row,
    runtime_error, shape_error,
};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::RemainingBudget { .. } => remaining_budget_value(input),
        Input::RemainingHours { .. } => remaining_hours_value(input),
        Input::Team { .. } => team_value(input),
        Input::Org { .. } => org_value(input),
        Input::Key { .. } => key_value(input),
        Input::User { .. } => user_value(input),
        Input::Customer { .. } => customer_value(input),
        Input::TeamMembers { .. } => team_members_value(input),
        Input::Provider { .. } => provider_value(input),
        Input::KeyList { .. } => key_list_value(input),
        Input::OrgList { .. } => org_list_value(input),
        Input::CustomerRow { .. } => customer_row_value(input),
    }
}

fn team_members_value(input: &Input) -> HelperResult {
    let Input::TeamMembers {
        team_id,
        team_alias,
        member_count,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let log = new_log();
    let gauge = field(TEAM_MEMBERS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = TeamMembersGaugeInput {
        team_id: team_id.clone(),
        team_alias: team_alias.clone(),
        member_count: *member_count,
    };
    set_team_members_metric(&gauge, &call, label_source(&settings, false))
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&samples(&log)))
}

fn provider_value(input: &Input) -> HelperResult {
    let Input::Provider {
        provider,
        spend,
        budget_limit,
    } = input
    else {
        return Err(shape_error());
    };
    let log = new_log();
    let gauge = GaugeField {
        metric_name: PROVIDER_REMAINING,
        supported_labels: vec![UserApiKeyLabelName::ApiProvider.as_str().to_owned()],
        collector: ProbeGauge::new(PROVIDER_REMAINING, &log),
    };
    track_provider_remaining_budget(&gauge, provider, money(*spend)?, money(*budget_limit)?);
    Ok(samples_value(&samples(&log)))
}

fn key_list_value(input: &Input) -> HelperResult {
    let Input::KeyList {
        entries,
        now,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(KEY_REMAINING, KEY_MAX, KEY_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let calls: Vec<KeyListEntry> = entries
        .iter()
        .map(key_list_entry)
        .collect::<Result<_, _>>()?;
    set_key_list_budget_metrics(&gauges, &calls, label_source(&settings, false), *now)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

fn org_list_value(input: &Input) -> HelperResult {
    let Input::OrgList {
        rows,
        now,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let log = new_log();
    let gauges = entity_gauges(ORG_REMAINING, ORG_MAX, ORG_HOURS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let calls: Vec<OrgBudgetGaugeRow> = rows.iter().map(org_row).collect::<Result<_, _>>()?;
    set_org_list_budget_metrics(&gauges, &calls, label_source(&settings, false), *now)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

fn customer_row_value(input: &Input) -> HelperResult {
    let Input::CustomerRow {
        customer,
        default_budget,
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
    let row = customer_row(customer)?;
    let defaults = budget_row(default_budget.as_ref())?;
    let context = customer_context(
        &settings,
        *enable_end_user_tracking,
        *now,
        &tracker,
        &limits,
    );
    set_customer_budget_metrics_from_row(&gauges, &row, defaults.as_ref(), context)
        .map_err(runtime_error)?;
    Ok(samples_value(&samples(&log)))
}

fn remaining_budget_value(input: &Input) -> HelperResult {
    let Input::RemainingBudget { max_budget, spend } = input else {
        return Err(shape_error());
    };
    let remaining = remaining_budget(money(*max_budget)?, money(*spend)?);
    Ok(json!({ "remaining": metric_text(remaining.metric_value()) }))
}

fn remaining_hours_value(input: &Input) -> HelperResult {
    let Input::RemainingHours { reset_at, now } = input else {
        return Err(shape_error());
    };
    let hours = remaining_hours_for_budget_reset(*reset_at, *now);
    Ok(json!({ "hours": format!("{hours:.6}") }))
}
