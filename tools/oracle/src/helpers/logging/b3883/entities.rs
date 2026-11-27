use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest, set_customer_budget_metrics_after_api_request,
};
use betterlite_logging::prometheus::budget_lookups::customer_default::get_default_customer_budget;
use betterlite_logging::prometheus::budget_lookups::key_budget::{
    KeyBudgetRequest, set_key_budget_metrics_after_api_request,
};
use betterlite_logging::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest, set_org_budget_metrics_after_api_request,
};
use betterlite_logging::prometheus::budget_lookups::snapshots::{
    KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use betterlite_logging::prometheus::budget_lookups::team_budget::{
    TeamBudgetRequest, set_team_budget_metrics_after_api_request,
};
use betterlite_logging::prometheus::budget_lookups::user_budget::{
    UserBudgetRequest, set_user_budget_metrics_after_api_request,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{Input, KeySnapshotCall, OrgSnapshotCall, TeamSnapshotCall, UserSnapshotCall};
use super::lookups::ScriptedLookups;
use super::output::budget_output;
use super::probe::ProbeLog;
use super::support::{
    CUSTOMER_GAUGES, KEY_GAUGES, ORG_GAUGES, TEAM_GAUGES, USER_GAUGES, customer_context,
    entity_gauges, finish, label_source, runtime, runtime_error, shape_error,
};

impl TeamSnapshotCall {
    const fn snapshot(&self) -> TeamBudgetSnapshot {
        TeamBudgetSnapshot {
            budget_reset_at: self.budget_reset_at,
            max_budget: self.max_budget,
        }
    }
}

impl OrgSnapshotCall {
    const fn snapshot(&self) -> OrgBudgetSnapshot {
        OrgBudgetSnapshot {
            spend: self.spend,
            max_budget: self.max_budget,
        }
    }
}

impl KeySnapshotCall {
    const fn snapshot(&self) -> KeyBudgetSnapshot {
        KeyBudgetSnapshot {
            budget_reset_at: self.budget_reset_at,
        }
    }
}

impl UserSnapshotCall {
    fn snapshot(&self) -> UserBudgetSnapshot {
        UserBudgetSnapshot {
            budget_reset_at: self.budget_reset_at,
            max_budget: self.max_budget,
            user_alias: self.user_alias.clone(),
        }
    }
}

pub(super) fn team_value(input: &Input) -> HelperResult {
    let Input::Team {
        team_id,
        team_alias,
        team_spend,
        team_max_budget,
        response_cost,
        carried,
        lookup,
        now,
        noop,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(TEAM_GAUGES, labels_by_metric, *noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(lookup.clone(), true);
    let carried_state = carried.as_ref().map(TeamSnapshotCall::snapshot);
    let request = TeamBudgetRequest {
        user_api_team: team_id.as_deref(),
        user_api_team_alias: team_alias.as_deref(),
        team_spend: *team_spend,
        team_max_budget: *team_max_budget,
        response_cost: *response_cost,
        carried: carried_state.as_ref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_team_budget_metrics_after_api_request(&scripted, &gauges, source, *now, &request),
    )
}

pub(super) fn org_value(input: &Input) -> HelperResult {
    let Input::Org {
        org_id,
        org_alias,
        response_cost,
        carried,
        database,
        lookup,
        now,
        noop,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(ORG_GAUGES, labels_by_metric, *noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(lookup.clone(), *database);
    let carried_state = carried.as_ref().map(OrgSnapshotCall::snapshot);
    let request = OrgBudgetRequest {
        org_id: org_id.as_deref(),
        response_cost: *response_cost,
        carried: carried_state.as_ref(),
        org_alias: org_alias.as_deref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_org_budget_metrics_after_api_request(&scripted, &gauges, source, *now, &request),
    )
}

pub(super) fn customer_value(input: &Input) -> HelperResult {
    let Input::Customer {
        end_user_id,
        response_cost,
        enable_prometheus_only,
        disable_end_user_cost_tracking,
        lookup,
        now,
        noop,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(CUSTOMER_GAUGES, labels_by_metric, *noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let end_user = EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: *disable_end_user_cost_tracking,
        enable_prometheus_only: *enable_prometheus_only,
    };
    let tracker = BoundedPrometheusSeriesTracker::default();
    let limits = SeriesLimits::default();
    let context = customer_context(&settings, end_user, *now, &tracker, &limits);
    let scripted = ScriptedLookups::new(lookup.clone(), true);
    let request = CustomerBudgetRequest {
        end_user_id: end_user_id.as_deref(),
        response_cost: *response_cost,
    };
    finish(
        &probe,
        &scripted,
        set_customer_budget_metrics_after_api_request(
            &scripted, &gauges, context, end_user, &request,
        ),
    )
}

pub(super) fn key_value(input: &Input) -> HelperResult {
    let Input::Key {
        user_api_key,
        user_api_key_alias,
        response_cost,
        key_max_budget,
        key_spend,
        carried,
        lookup,
        now,
        noop,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(KEY_GAUGES, labels_by_metric, *noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(lookup.clone(), true);
    let carried_state = carried.as_ref().map(KeySnapshotCall::snapshot);
    let request = KeyBudgetRequest {
        user_api_key: user_api_key.as_deref(),
        user_api_key_alias: user_api_key_alias.as_deref(),
        response_cost: *response_cost,
        key_max_budget: *key_max_budget,
        key_spend: *key_spend,
        carried: carried_state.as_ref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_key_budget_metrics_after_api_request(&scripted, &gauges, source, *now, &request),
    )
}

pub(super) fn user_value(input: &Input) -> HelperResult {
    let Input::User {
        user_id,
        user_spend,
        user_max_budget,
        response_cost,
        carried,
        user_email,
        lookup,
        now,
        noop,
        labels_by_metric,
    } = input
    else {
        return Err(shape_error());
    };
    let probe = ProbeLog::default();
    let gauges = entity_gauges(USER_GAUGES, labels_by_metric, *noop, &probe)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let scripted = ScriptedLookups::new(lookup.clone(), true);
    let carried_state = carried.as_ref().map(UserSnapshotCall::snapshot);
    let request = UserBudgetRequest {
        user_id: user_id.as_deref(),
        user_spend: *user_spend,
        user_max_budget: *user_max_budget,
        response_cost: *response_cost,
        carried: carried_state.as_ref(),
        user_email: user_email.as_deref(),
    };
    let source = label_source(&settings, None, Some(true));
    finish(
        &probe,
        &scripted,
        set_user_budget_metrics_after_api_request(&scripted, &gauges, source, *now, &request),
    )
}

pub(super) fn default_value(input: &Input) -> HelperResult {
    let Input::DefaultCustomerBudget {
        default_budget_id,
        lookup,
    } = input
    else {
        return Err(shape_error());
    };
    let scripted = ScriptedLookups::new(lookup.clone(), true);
    let row: Option<BudgetTableSnapshot> = runtime()?.block_on(get_default_customer_budget(
        &scripted,
        default_budget_id.as_deref(),
    ));
    Ok(budget_output(row, &scripted.calls()))
}
