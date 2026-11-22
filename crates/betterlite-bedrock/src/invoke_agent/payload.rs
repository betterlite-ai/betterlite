use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::{Message, convert_content_list_to_str};
use indexmap::IndexMap;

use super::error::InvokeAgentRequestError;
use crate::model_routes::ParamMap;
use crate::model_routes::ParamValue;

const INPUT_TEXT_FIELD: &str = "inputText";
const ENABLE_TRACE_FIELD: &str = "enableTrace";

pub fn transform_request(
    messages: &[Message],
    optional_params: &ParamMap,
) -> Result<IndexMap<String, Json>, InvokeAgentRequestError> {
    let last = messages
        .last()
        .ok_or(InvokeAgentRequestError::EmptyMessages)?;
    let mut body = IndexMap::new();
    body.insert(
        INPUT_TEXT_FIELD.to_owned(),
        Json::Text(convert_content_list_to_str(last)),
    );
    body.insert(ENABLE_TRACE_FIELD.to_owned(), Json::Bool(true));
    for (name, value) in optional_params {
        body.insert(name.clone(), request_value(value)?);
    }
    Ok(body)
}

// A text round-trip keeps object keys in the caller's order, which serde_json's Value would sort
fn request_value(value: &ParamValue) -> Result<Json, InvokeAgentRequestError> {
    let encoded = serde_json::to_string(value)
        .map_err(|source| InvokeAgentRequestError::Parameters { source })?;
    serde_json::from_str(&encoded).map_err(|source| InvokeAgentRequestError::Parameters { source })
}
