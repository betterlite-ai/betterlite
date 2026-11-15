use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::api::container_contracts::error::ContainerError;

// model_validate takes only an object and keeps a repeated key's last value; a derived record also takes an array
pub fn decode_container_reply<T>(bytes: &[u8]) -> Result<T, ContainerError>
where
    T: DeserializeOwned,
{
    let fields: Map<String, Value> =
        serde_json::from_slice(bytes).map_err(|error| ContainerError::Decode(error.to_string()))?;
    serde_json::from_value(Value::Object(fields))
        .map_err(|error| ContainerError::Decode(error.to_string()))
}
