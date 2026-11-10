pub mod converse;
pub mod endpoint;
pub mod error;
pub mod invoke;
pub mod request;
pub mod response;
mod tests;
pub mod validate;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use request::CountTokensRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputType {
    Converse,
    InvokeModel,
}

#[must_use]
pub fn detect_input_type(request: &IndexMap<String, Json>) -> InputType {
    let Some(Json::List(messages)) = request.get("messages") else {
        return InputType::InvokeModel;
    };
    for message in messages {
        let Json::Object(fields) = message else {
            continue;
        };
        let Some(Json::List(content)) = fields.get("content") else {
            continue;
        };
        let typed = content.iter().any(|block| {
            matches!(block, Json::Object(block_fields) if block_fields.contains_key("type"))
        });
        if typed {
            return InputType::InvokeModel;
        }
    }
    InputType::Converse
}

#[must_use]
pub fn transform_request(request: &IndexMap<String, Json>) -> CountTokensRequest {
    match detect_input_type(request) {
        InputType::Converse => converse::transform_converse_request(request),
        InputType::InvokeModel => invoke::transform_invoke_model_request(request),
    }
}
