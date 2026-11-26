use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};

use super::Call;

pub(super) fn token_of(call: &Call) -> Result<UserAPIKeyAuth, Failure> {
    let fields = call
        .token
        .clone()
        .ok_or(Failure::Missing("the token fields"))?;
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(Failure::Auth)
}

// a limiter failure that is not a budget error is not one LiteLLM records
pub(super) fn error_value(outcome: Result<(), ErrorClass>) -> Result<Value, Failure> {
    match outcome {
        Ok(()) => Ok(Value::Null),
        Err(ErrorClass::BudgetExceeded(details)) => Ok(denial_json(&details)),
        Err(_other) => Err(Failure::Limiter),
    }
}

// BudgetExceededError's caller-visible fields, with the status litellm.budget_exceeded_status_code defaults to
pub(super) fn denial_json(details: &BudgetExceededDetails) -> Value {
    json!({
        "exception": "BudgetExceededError",
        "status": details.status_code,
        "current_cost": decimal_json(details.current_cost),
        "max_budget": decimal_json(details.max_budget),
        "entity_type": details.entity_type,
        "entity_id": details.entity_id,
    })
}

pub(super) fn decimal_json(value: Decimal) -> Value {
    value
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Failure {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the limiter failed")]
    Limiter,
    #[error("the global spend read failed")]
    Coordinator,
    #[error("the call does not name {0}")]
    Missing(&'static str),
}
