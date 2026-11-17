use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_delegation::budget::check_budget_ceilings;
use betterlite_server::key_delegation::caller::Caller;
use betterlite_server::key_delegation::error::{DelegationDetail, DelegationError};
use betterlite_server::key_delegation::ownership::validate_ownership_change;
use betterlite_server::key_delegation::permissions::check_permissions_access;
use betterlite_server::key_delegation::request::{KeyDelegationRequest, Sent};
use betterlite_server::key_delegation::routes::{
    check_routes_permission, enforce_routes_update, is_safe_route_transition,
};
use betterlite_server::key_fields::base::KeyRequestBase;
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::management_guards::amounts::Amount;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "check", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    Ownership {
        body: JsonData,
        existing_user_id: Option<String>,
        caller: CallerInput,
    },
    Routes {
        allowed_routes: Option<Vec<JsonData>>,
        was_provided: bool,
        allow_safe_presets: bool,
        caller: CallerInput,
    },
    Transition {
        incoming: Option<Vec<JsonData>>,
        existing: Option<Vec<JsonData>>,
    },
    RoutesUpdate {
        body: JsonData,
        existing_allowed_routes: Option<Vec<JsonData>>,
        caller: CallerInput,
    },
    Permissions {
        body: JsonData,
        caller: CallerInput,
    },
    Budget {
        budget_limits: Option<Vec<WindowInput>>,
        delegation_ceiling: Option<JsonData>,
        caller: CallerInput,
        has_team: bool,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallerInput {
    role: Option<UserRole>,
    is_session_token: bool,
    is_ui_session_team_key: bool,
}

// LiteLLM's BudgetLimitEntry also carries budget_duration; the check never reads it.
#[derive(Deserialize)]
pub(crate) struct WindowInput {
    max_budget: JsonData,
}

impl CallerInput {
    const fn to_caller(&self) -> Caller {
        Caller {
            role: self.role,
            is_session_token: self.is_session_token,
            is_ui_session_team_key: self.is_ui_session_team_key,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(call_outcome)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!(outcomes))
    }
}

fn call_outcome(call: &Call) -> HelperResult {
    match call {
        Call::Ownership {
            body,
            existing_user_id,
            caller,
        } => ownership_outcome(body, existing_user_id.as_deref(), caller),
        Call::Routes {
            allowed_routes,
            was_provided,
            allow_safe_presets,
            caller,
        } => {
            let routes = Sent {
                supplied: *was_provided,
                value: allowed_routes.clone(),
            };
            routes_outcome(&routes, *allow_safe_presets, caller)
        }
        Call::Transition { incoming, existing } => {
            transition_outcome(incoming.as_deref(), existing.as_deref())
        }
        Call::RoutesUpdate {
            body,
            existing_allowed_routes,
            caller,
        } => routes_update_outcome(body, existing_allowed_routes.as_deref(), caller),
        Call::Permissions { body, caller } => permissions_outcome(body, caller),
        Call::Budget {
            budget_limits,
            delegation_ceiling,
            caller,
            has_team,
        } => budget_outcome(
            budget_limits.as_deref(),
            delegation_ceiling.as_ref(),
            caller,
            *has_team,
        ),
    }
}

fn ownership_outcome(
    body: &JsonData,
    existing_user_id: Option<&str>,
    caller: &CallerInput,
) -> HelperResult {
    let request = projected(body)?;
    outcome(validate_ownership_change(
        Some(&request),
        existing_user_id,
        caller.to_caller(),
    ))
}

fn routes_outcome(
    routes: &Sent<Vec<JsonData>>,
    allow_safe_presets: bool,
    caller: &CallerInput,
) -> HelperResult {
    outcome(check_routes_permission(
        routes,
        allow_safe_presets,
        caller.to_caller(),
    ))
}

fn transition_outcome(
    incoming: Option<&[JsonData]>,
    existing: Option<&[JsonData]>,
) -> HelperResult {
    Ok(json!({
        "ok": true,
        "value": is_safe_route_transition(incoming, existing),
    }))
}

fn routes_update_outcome(
    body: &JsonData,
    existing_allowed_routes: Option<&[JsonData]>,
    caller: &CallerInput,
) -> HelperResult {
    let request = projected(body)?;
    outcome(enforce_routes_update(
        &request.allowed_routes,
        existing_allowed_routes,
        caller.to_caller(),
    ))
}

fn permissions_outcome(body: &JsonData, caller: &CallerInput) -> HelperResult {
    let request = projected(body)?;
    outcome(check_permissions_access(
        &request.permissions,
        caller.to_caller(),
    ))
}

fn budget_outcome(
    budget_limits: Option<&[WindowInput]>,
    delegation_ceiling: Option<&JsonData>,
    caller: &CallerInput,
    has_team: bool,
) -> HelperResult {
    let limits = window_amounts(budget_limits)?;
    let ceiling = delegation_ceiling.map(amount).transpose()?;
    outcome(check_budget_ceilings(
        limits.as_deref(),
        ceiling,
        caller.to_caller(),
        has_team,
    ))
}

fn projected(body: &JsonData) -> Result<KeyDelegationRequest, Box<HelperError>> {
    let value = TypedValue { json: body.clone() };
    let request = KeyRequestBase::read(&value).or_value_error()?;
    Ok(KeyDelegationRequest::from(&request))
}

fn window_amounts(
    budget_limits: Option<&[WindowInput]>,
) -> Result<Option<Vec<Amount>>, Box<HelperError>> {
    budget_limits
        .map(|windows| {
            windows
                .iter()
                .map(|window| amount(&window.max_budget))
                .collect()
        })
        .transpose()
}

fn amount(value: &JsonData) -> Result<Amount, Box<HelperError>> {
    let parsed = match value {
        JsonData::Number(number) => amount_from_text(&number.to_string()).map(Amount::Finite),
        JsonData::String(text) => match text.as_str() {
            "nan" => Some(Amount::Nan),
            "inf" | "infinity" => Some(Amount::PositiveInfinity),
            "-inf" | "-infinity" => Some(Amount::NegativeInfinity),
            _ => None,
        },
        JsonData::Null | JsonData::Bool(_) | JsonData::Array(_) | JsonData::Object(_) => None,
    };
    parsed.ok_or_else(|| invalid("an amount is a number or one of nan, inf, -inf"))
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn outcome(result: Result<(), DelegationError>) -> HelperResult {
    match result {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(failure) => {
            let detail_error = matches!(failure.detail(), DelegationDetail::Error(_));
            Ok(json!({
                "ok": false,
                "status": failure.status_code().as_u16(),
                "detail_error": detail_error,
            }))
        }
    }
}
