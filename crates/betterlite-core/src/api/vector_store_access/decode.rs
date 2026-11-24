use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};

use super::error::VectorStoreAccessError;

// stored litellm_params text decoded at the boundary: its object, or an empty one
pub fn decode_vector_store_access(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<Json, VectorStoreAccessError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    Ok(Json::Object(parse_params(bytes)))
}

// json.loads followed by isinstance(parsed, dict): malformed JSON and every non-object read as {}
#[must_use]
pub(super) fn parse_params(bytes: &[u8]) -> IndexMap<String, Json> {
    match serde_json::from_slice::<Json>(bytes) {
        Ok(Json::Object(fields)) => fields,
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => IndexMap::new(),
    }
}
