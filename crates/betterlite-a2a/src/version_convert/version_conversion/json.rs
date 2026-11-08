use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::version_convert::{JsonObject, to_object, to_value};

use super::error::ConvertError;

pub(super) fn decode<T>(value: &JsonObject) -> Result<T, ConvertError>
where
    T: DeserializeOwned,
{
    match serde_json::from_value(to_value(value)) {
        Ok(decoded) => Ok(decoded),
        Err(source) => Err(ConvertError::Decode(source)),
    }
}

pub(super) fn encode<T>(value: &T) -> Result<JsonObject, ConvertError>
where
    T: Serialize,
{
    to_object(&serde_json::to_value(value).map_err(ConvertError::Encode)?)
        .ok_or(ConvertError::NotAnObject)
}
