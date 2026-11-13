use betterlite_core::agents::json::AgentJson;
use serde::Serialize;

use crate::version_convert::JsonObject;

use super::error::ConvertError;

pub(super) fn encode<T>(value: &T) -> Result<JsonObject, ConvertError>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(ConvertError::Encode)?;
    let AgentJson::Object(fields) = encoded else {
        return Err(ConvertError::NotAnObject);
    };
    Ok(fields.into_iter().collect())
}
