use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::EventPayloadError;

// The flag stash_block_context sets on model_call_details so the success handler logs nothing for a blocked request.
pub const BLOCKED_FLAG: &str = "_rubrik_blocked";

pub fn field_object<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'fields IndexMap<String, Json>>, EventPayloadError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(object)) => Ok(Some(object)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(EventPayloadError::FieldType { field: name })
        }
    }
}
