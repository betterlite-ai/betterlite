use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{DecodeError, SafetyError, TypedValue, decode_request_metadata};

// _coerce_metadata_to_dict: a dict stays, a JSON string parses, and anything else or malformed is None.
pub fn coerce(value: Option<&Json>) -> Result<Option<JsonObject>, SafetyError> {
    let text = match value {
        Some(Json::Object(fields)) => return Ok(Some(fields.clone())),
        Some(Json::Text(text)) => text,
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Ok(None);
        }
    };
    match decode_request_metadata(text.as_bytes()) {
        Ok(TypedValue {
            json: Json::Object(fields),
        }) => Ok(Some(fields)),
        Ok(TypedValue {
            json: Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        }) => Ok(None),
        Err(error @ (DecodeError::TooLarge { .. } | DecodeError::TooDeep { .. })) => {
            Err(SafetyError::MetadataLimit {
                error: Box::new(error),
            })
        }
        Err(DecodeError::Malformed { .. }) => Ok(None),
    }
}
