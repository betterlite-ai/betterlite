use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{DecodeError, SafetyError, TypedValue, decode_request_metadata};

// LiteLLM's _coerce_metadata_to_dict: an object stays as it is and a JSON string is parsed. Anything else
// is None, and a malformed string is None like safe_json_loads' default. A string past the shared byte or
// depth bound is refused instead of ignored, which is the bound this new outside-byte decoder records.
pub fn coerce(value: Option<&Json>) -> Result<Option<JsonObject>, SafetyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields.clone())),
        Some(Json::Text(text)) => coerce_string(text),
        Some(Json::Bool(_)) | Some(Json::Number(_)) | Some(Json::List(_)) => Ok(None),
    }
}

fn coerce_string(text: &str) -> Result<Option<JsonObject>, SafetyError> {
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
