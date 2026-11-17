use percent_encoding::percent_decode_str;
use serde_json::Value;

use super::error::FireworksResponsesError;
use crate::api::responses_shapes::DeleteResponseResult;
use crate::openai_base_config::Object;

// LiteLLM takes httpx's already decoded URL path and unquotes the last segment again, so the raw path is decoded twice.
pub fn transform_delete_response(
    path: &str,
) -> Result<DeleteResponseResult, FireworksResponsesError> {
    let decoded = percent_decode_str(path).decode_utf8_lossy();
    let segment = decoded.rsplit('/').next().unwrap_or_default();
    let id = percent_decode_str(segment).decode_utf8_lossy();
    let mut body = Object::new();
    body.insert(String::from("id"), Value::String(id.into_owned()));
    body.insert(
        String::from("object"),
        Value::String(String::from("response")),
    );
    body.insert(String::from("deleted"), Value::Bool(true));
    serde_json::from_value(Value::Object(body))
        .map_err(|source| FireworksResponsesError::Delete { source })
}
