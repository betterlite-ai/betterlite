use betterlite_models::decisions::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::SyntheticEventsError;

pub fn to_json<T: Serialize>(value: &T) -> Result<Json, SyntheticEventsError> {
    let serialized = serde_json::to_value(value)?;
    Ok(serde_json::from_value(serialized)?)
}

pub fn from_json<T: DeserializeOwned>(value: &Json) -> Result<T, SyntheticEventsError> {
    let serialized = serde_json::to_value(value)?;
    Ok(serde_json::from_value(serialized)?)
}
