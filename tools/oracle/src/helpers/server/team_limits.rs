use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::model_budget_values::budget_config::BudgetRuntime;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::team_limits::{DefaultTeamParams, Settings};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::values::Amount;
use betterlite_server::team_limits::defaults::{
    Caller, TeamParamField, default_team_param, is_available_team, should_auto_add_team_creator,
};
use betterlite_server::team_limits::durations::ZoneDurations;
use betterlite_server::team_limits::guard::budget_update_authority;
use betterlite_server::team_limits::model_cap::model_budget_authority;
use betterlite_server::team_request_contracts::update::UpdateTeamRequest;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

// the default params dwarf the other variants, so they are boxed to stay within 3x, the limit variant_size_differences enforces
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    DefaultTeamParam {
        field: TeamParamField,
        params: Option<Box<DefaultTeamParams>>,
    },
    AvailableTeam {
        team_id: String,
        available_teams: Option<Vec<String>>,
        user_id: Option<String>,
        user_role: Option<UserRole>,
    },
    CreatorAutoAdd {
        user_id: Option<String>,
        user_role: Option<UserRole>,
        disable: bool,
    },
    BudgetGuard {
        request: Box<JsonData>,
        user_role: Option<UserRole>,
        existing: Option<Amount>,
    },
    ModelCap {
        request: Box<JsonData>,
        user_role: Option<UserRole>,
        existing: Option<BTreeMap<String, JsonData>>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DefaultTeamParam { field, params } => {
                Ok(default_team_param_answer(*field, params.as_deref()))
            }
            Input::AvailableTeam {
                team_id,
                available_teams,
                user_id,
                user_role,
            } => Ok(available_team_answer(
                team_id,
                available_teams.as_deref(),
                user_id.as_deref(),
                *user_role,
            )),
            Input::CreatorAutoAdd {
                user_id,
                user_role,
                disable,
            } => Ok(creator_auto_add_answer(
                user_id.as_deref(),
                *user_role,
                *disable,
            )),
            Input::BudgetGuard {
                request,
                user_role,
                existing,
            } => budget_guard(request, *user_role, *existing),
            Input::ModelCap {
                request,
                user_role,
                existing,
            } => model_cap(request, *user_role, existing.as_ref()),
        }
    }
}

fn default_team_param_answer(field: TeamParamField, params: Option<&DefaultTeamParams>) -> Value {
    let settings = Settings {
        default_team_params: params.cloned(),
        disable_admin_auto_add: false,
    };
    json!({"value": default_team_param(&settings, field)})
}

fn available_team_answer(
    team_id: &str,
    available_teams: Option<&[String]>,
    user_id: Option<&str>,
    user_role: Option<UserRole>,
) -> Value {
    let caller = Caller {
        user_id: user_id.map(str::to_owned),
        user_role,
    };
    json!({"available": is_available_team(team_id, &caller, available_teams)})
}

fn creator_auto_add_answer(
    user_id: Option<&str>,
    user_role: Option<UserRole>,
    disable: bool,
) -> Value {
    let settings = Settings {
        default_team_params: None,
        disable_admin_auto_add: disable,
    };
    let caller = Caller {
        user_id: user_id.map(str::to_owned),
        user_role,
    };
    json!({"auto_add": should_auto_add_team_creator(&caller, &settings)})
}

fn budget_guard(
    request: &JsonData,
    user_role: Option<UserRole>,
    existing: Option<Amount>,
) -> HelperResult {
    let submitted = UpdateTeamRequest::from_value(request).or_value_error()?;
    let caller = Caller {
        user_id: None,
        user_role,
    };
    let outcome = budget_update_authority(&submitted, &caller, existing);
    let (status, guarded, expected): (Option<u16>, bool, Option<Amount>) = match outcome {
        Ok(Some(guard)) => (None, true, guard.expected),
        Ok(None) => (None, false, None),
        Err(_) => (Some(StatusCode::FORBIDDEN.as_u16()), false, None),
    };
    Ok(json!({"status": status, "guarded": guarded, "expected": expected}))
}

fn model_cap(
    request: &JsonData,
    user_role: Option<UserRole>,
    existing: Option<&BTreeMap<String, JsonData>>,
) -> HelperResult {
    let submitted = UpdateTeamRequest::from_value(request).or_value_error()?;
    let caller = Caller {
        user_id: None,
        user_role,
    };
    let database = TimeZoneDatabase::from_env();
    let durations = ZoneDurations {
        database: &database,
    };
    let runtime = BudgetRuntime {
        durations: &durations,
        now: DateTime::UNIX_EPOCH,
    };
    let outcome = model_budget_authority(&submitted, &caller, existing, &NoBedrock, runtime);
    let status = outcome.err().map(|_| StatusCode::FORBIDDEN.as_u16());
    Ok(json!({"status": status}))
}

// the cases stay on models litellm.model_cost does not price as bedrock, so no candidate comes from a provider prefix
struct NoBedrock;

impl BedrockCatalog for NoBedrock {
    fn base_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn prices_as_bedrock(&self, _base_model: &str) -> bool {
        false
    }
}
