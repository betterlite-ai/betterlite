use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_customer_budget_metrics_after_api_request as set_customer_budget,
};
use betterlite_logging::prometheus::budget_lookups::customer_default::get_default_customer_budget;
use betterlite_logging::prometheus::budget_lookups::key_budget::{
    KeyBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_key_budget_metrics_after_api_request as set_key_budget,
};
use betterlite_logging::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_org_budget_metrics_after_api_request as set_org_budget,
};
use betterlite_logging::prometheus::budget_lookups::snapshots::{
    KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use betterlite_logging::prometheus::budget_lookups::team_budget::{
    TeamBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_team_budget_metrics_after_api_request as set_team_budget,
};
use betterlite_logging::prometheus::budget_lookups::user_budget::{
    UserBudgetRequest,
    // ast-grep-ignore: no-long-names
    set_user_budget_metrics_after_api_request as set_user_budget,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;
use rust_decimal::Decimal;

use super::input::Input;
use super::lookups::ScriptedLookups;
use super::output::budget_output;
use super::probe::ProbeLog;
use super::support::{
    CUSTOMER_GAUGES, KEY_GAUGES, ORG_GAUGES, TEAM_GAUGES, USER_GAUGES, customer_context,
    entity_gauges, finish, label_source, runtime, shape_error,
};

pub(super) fn team_value(input: &Input) -> HelperResult {
    let now = input.now.ok_or_else(shape_error)?;
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

pub(super) fn org_value(input: &Input) -> HelperResult {
    let now = input.now.ok_or_else(shape_error)?;
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

pub(super) fn customer_value(input: &Input) -> HelperResult {
    let now = input.now.ok_or_else(shape_error)?;
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

pub(super) fn key_value(input: &Input) -> HelperResult {
    let now = input.now.ok_or_else(shape_error)?;
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

pub(super) fn user_value(input: &Input) -> HelperResult {
    let now = input.now.ok_or_else(shape_error)?;
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

pub(super) fn default_value(input: &Input) -> HelperResult {
    let scripted = ScriptedLookups::new(input.lookup.clone(), true);
    let row: Option<BudgetTableSnapshot> = runtime()?.block_on(get_default_customer_budget(
        &scripted,
        input.default_budget_id.as_deref(),
    ));
    Ok(budget_output(row, &scripted.calls()))
}
