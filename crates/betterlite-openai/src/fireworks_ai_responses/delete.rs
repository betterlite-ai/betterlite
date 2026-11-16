use percent_encoding::percent_decode_str;
use serde_json::json;

use super::error::FireworksResponsesError;
use crate::api::responses_shapes::DeleteResponseResult;

// LiteLLM takes httpx's already decoded URL path and unquotes the last segment again, so the raw path is decoded twice.
pub fn transform_delete_response(
    path: &str,
) -> Result<DeleteResponseResult, FireworksResponsesError> {
    let decoded = percent_decode_str(path).decode_utf8_lossy();
    let segment = decoded.rsplit('/').next().unwrap_or_default();
    let id = percent_decode_str(segment).decode_utf8_lossy();
    let body = json!({"id": id.as_ref(), "object": "response", "deleted": true});
    serde_json::from_value(body).map_err(|source| FireworksResponsesError::Delete { source })
}
