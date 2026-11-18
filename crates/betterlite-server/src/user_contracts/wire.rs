use serde::Deserialize;
use serde::de::Error as _;

use crate::proxy_request_contracts::{ContractError, Fields};

pub(crate) fn wrong_type(field: &str) -> ContractError {
    ContractError::Validation {
        message: format!("{field} has a value of the wrong JSON type"),
    }
}

pub(crate) fn check_count(field: &str, count: usize, maximum: usize) -> Result<(), ContractError> {
    if count == 0 || count > maximum {
        return Err(ContractError::from(format!(
            "{field} must hold between 1 and {maximum} values"
        )));
    }
    Ok(())
}

pub(crate) fn reject_unknown_fields(
    fields: &Fields,
    allowed: &[&str],
) -> Result<(), ContractError> {
    for name in fields.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(ContractError::from(format!(
                "extra inputs are not permitted: `{name}`"
            )));
        }
    }
    Ok(())
}

// the mapping's value type is not optional, so an explicit null fails where an absent key is allowed
pub(crate) fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Option::<T>::deserialize(deserializer)?;
    let inner = value.ok_or_else(|| D::Error::custom("null is not a mapping value"))?;
    Ok(Some(inner))
}
