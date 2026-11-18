use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::de::Error as _;

use crate::key_fields::values::Amount;

pub mod budget;
pub mod coordination_redis;
pub mod cost_estimate;
pub mod customer;
pub mod key;
pub mod user;

mod tests;

pub(crate) type Fields = BTreeMap<String, JsonData>;

pub(crate) const fn default_amount() -> Amount {
    Amount(Decimal::ZERO)
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("{message}")]
    Validation { message: String },
    #[error(transparent)]
    Decode {
        #[from]
        source: serde_json::Error,
    },
}

impl From<&str> for ContractError {
    fn from(message: &str) -> Self {
        Self::Validation {
            message: message.to_owned(),
        }
    }
}

impl From<String> for ContractError {
    fn from(message: String) -> Self {
        Self::Validation { message }
    }
}

pub(crate) fn require_object(value: &JsonValue) -> Result<Fields, ContractError> {
    let data = serde_json::from_value::<JsonData>(value.clone()).map_err(ContractError::from)?;
    let JsonData::Object(fields) = data else {
        return Err(not_an_object());
    };
    Ok(fields)
}

fn not_an_object() -> ContractError {
    let message = "the request body is not a JSON object";
    ContractError::Decode {
        source: serde_json::Error::custom(message),
    }
}

pub(crate) fn missing_field(name: &str) -> ContractError {
    let message = format!("missing field `{name}`");
    ContractError::Decode {
        source: serde_json::Error::custom(message),
    }
}

pub(crate) fn require_present(fields: &Fields, names: &[&str]) -> Result<(), ContractError> {
    for name in names {
        if !fields.contains_key(*name) {
            return Err(missing_field(name));
        }
    }
    Ok(())
}

// pydantic tests a before-validator's input for truthiness, so a missing key, a null and an empty list all fail the same check
pub(crate) fn is_missing_or_empty(fields: &Fields, name: &str) -> bool {
    match fields.get(name) {
        None | Some(JsonData::Null) => true,
        Some(JsonData::Array(items)) => items.is_empty(),
        Some(_) => false,
    }
}

// a before-validator that tests ``is not None'' accepts a zero, an empty string and an empty list alike
pub(crate) fn is_set(fields: &Fields, name: &str) -> bool {
    let Some(found) = fields.get(name) else {
        return false;
    };
    *found != JsonData::Null
}

pub(crate) fn supplied_fields(fields: &Fields, names: &[&str]) -> BTreeSet<String> {
    let mut supplied = BTreeSet::new();
    for name in names {
        if fields.contains_key(*name) {
            supplied.insert((*name).to_owned());
        }
    }
    supplied
}

pub(crate) fn decode<T>(fields: &Fields) -> Result<T, ContractError>
where
    T: DeserializeOwned,
{
    let document =
        serde_json::to_value(fields).map_err(|source| ContractError::Decode { source })?;
    let outcome: Result<T, serde_path_to_error::Error<serde_json::Error>> =
        serde_path_to_error::deserialize(document);
    outcome.map_err(with_path)
}

fn with_path(error: serde_path_to_error::Error<serde_json::Error>) -> ContractError {
    let path = error.path().to_string();
    let source = error.into_inner();
    if path == "." {
        return ContractError::Decode { source };
    }
    let message = format!("{path}: {source}");
    ContractError::Decode {
        source: serde_json::Error::custom(message),
    }
}
