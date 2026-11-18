use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

use super::{ContractError, Fields, decode, is_missing_or_empty, require_object};
use crate::key_fields::values::Amount;

const AT_LEAST_ONE: &str = "At least one of 'keys' or 'key_aliases' must be provided.";
const BOOLEAN_RESET: &str = "reset_to must be a number, not a boolean";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct KeyRequest {
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    #[serde(default)]
    pub key_aliases: Option<Vec<String>>,
}

impl KeyRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        check_at_least_one(&fields)?;
        decode(&fields)
    }
}

// The before-validator runs on the raw body, so an explicit null and a missing key fail the same way.
fn check_at_least_one(fields: &Fields) -> Result<(), ContractError> {
    if is_missing_or_empty(fields, "keys") && is_missing_or_empty(fields, "key_aliases") {
        return Err(ContractError::from(AT_LEAST_ONE));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResetSpendRequest {
    pub reset_to: Amount,
}

impl ResetSpendRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        reject_boolean(&fields)?;
        decode(&fields)
    }
}

// bool is an int in Python, so pydantic would turn true into 1.0; the guard rejects it before any number decoding.
fn reject_boolean(fields: &Fields) -> Result<(), ContractError> {
    let Some(JsonData::Bool(_)) = fields.get("reset_to") else {
        return Ok(());
    };
    Err(ContractError::from(BOOLEAN_RESET))
}

mod tests;
