use betterlite_auth::model_budget_values::amount_text::amount_from_text;
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
use serde_json::{Number, Value, json};

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
        caller: Caller,
    },
    Routes {
        allowed_routes: Option<Vec<JsonData>>,
        was_provided: bool,
        allow_safe_presets: bool,
        caller: Caller,
    },
    Transition {
        incoming: Option<Vec<JsonData>>,
        existing: Option<Vec<JsonData>>,
    },
    RoutesUpdate {
        body: JsonData,
        existing_allowed_routes: Option<Vec<JsonData>>,
        caller: Caller,
    },
    Permissions {
        body: JsonData,
        caller: Caller,
    },
    Budget {
        budget_limits: Option<Vec<WindowInput>>,
        delegation_ceiling: Option<AmountInput>,
        caller: Caller,
        has_team: bool,
    },
}

// LiteLLM's BudgetLimitEntry: budget_duration is required and max_budget carries the amount.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowInput {
    #[serde(rename = "budget_duration")]
    _budget_duration: String,
    max_budget: AmountInput,
}

// A JSON number keeps its exact decimal text; the strings are the non-finite markers LiteLLM accepts.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum AmountInput {
    Number(Number),
    Marker(NonFiniteMarker),
}

#[derive(Deserialize)]
pub(crate) enum NonFiniteMarker {
    #[serde(rename = "nan")]
    Nan,
    #[serde(rename = "inf")]
    Inf,
    #[serde(rename = "-inf")]
    NegativeInf,
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
        } => row_outcome(
            body,
            &ExistingSide::UserId(existing_user_id.clone()),
            *caller,
        ),
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
            Ok(outcome(check_routes_permission(
                &routes,
                *allow_safe_presets,
                *caller,
            )))
        }
        Call::Transition { incoming, existing } => {
            Ok(transition_outcome(incoming.as_deref(), existing.as_deref()))
        }
        Call::RoutesUpdate {
            body,
            existing_allowed_routes,
            caller,
        } => row_outcome(
            body,
            &ExistingSide::AllowedRoutes(existing_allowed_routes.clone()),
            *caller,
        ),
        Call::Permissions { body, caller } => permissions_outcome(body, *caller),
        Call::Budget {
            budget_limits,
            delegation_ceiling,
            caller,
            has_team,
        } => budget_outcome(
            budget_limits.as_deref(),
            delegation_ceiling.as_ref(),
            *caller,
            *has_team,
        ),
    }
}

// The row value each row-based check compares the request against.
enum ExistingSide {
    UserId(Option<String>),
    AllowedRoutes(Option<Vec<JsonData>>),
}

fn row_outcome(body: &JsonData, existing: &ExistingSide, caller: Caller) -> HelperResult {
    let request = projected(body)?;
    let checked = match existing {
        ExistingSide::UserId(user_id) => {
            validate_ownership_change(Some(&request), user_id.as_deref(), caller)
        }
        ExistingSide::AllowedRoutes(routes) => {
            enforce_routes_update(&request.allowed_routes, routes.as_deref(), caller)
        }
    };
    Ok(outcome(checked))
}

fn transition_outcome(incoming: Option<&[JsonData]>, existing: Option<&[JsonData]>) -> Value {
    json!({
        "ok": true,
        "value": is_safe_route_transition(incoming, existing),
    })
}

fn permissions_outcome(body: &JsonData, caller: Caller) -> HelperResult {
    let request = projected(body)?;
    Ok(outcome(check_permissions_access(
        &request.permissions,
        caller,
    )))
}

fn budget_outcome(
    budget_limits: Option<&[WindowInput]>,
    delegation_ceiling: Option<&AmountInput>,
    caller: Caller,
    has_team: bool,
) -> HelperResult {
    let limits = window_amounts(budget_limits)?;
    let ceiling = delegation_ceiling.map(amount).transpose()?;
    Ok(outcome(check_budget_ceilings(
        limits.as_deref(),
        ceiling,
        caller,
        has_team,
    )))
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

fn amount(value: &AmountInput) -> Result<Amount, Box<HelperError>> {
    match value {
        AmountInput::Number(number) => amount_from_text(&number.to_string())
            .map(Amount::Finite)
            .ok_or_else(|| invalid("an amount is a number or one of nan, inf, -inf")),
        AmountInput::Marker(NonFiniteMarker::Nan) => Ok(Amount::Nan),
        AmountInput::Marker(NonFiniteMarker::Inf) => Ok(Amount::PositiveInfinity),
        AmountInput::Marker(NonFiniteMarker::NegativeInf) => Ok(Amount::NegativeInfinity),
    }
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

fn outcome(result: Result<(), DelegationError>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(failure) => {
            let detail_error = matches!(failure.detail(), DelegationDetail::Error(_));
            json!({
                "ok": false,
                "status": failure.status_code().as_u16(),
                "detail_error": detail_error,
            })
        }
    }
}
