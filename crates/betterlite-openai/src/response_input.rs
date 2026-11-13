use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

pub mod error;
mod input;
mod prepare;
mod reasoning;
pub mod schema;
mod tools;

mod tests;

use error::ResponseInputError;
use prepare::prepared_input_and_tools;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<ResponsesItem>),
}

// The Responses item union is provider-defined, so an item keeps every field it arrived with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResponsesItem(pub Object);

// A tool entry is a nested namespace record or any non-object value LiteLLM forwards unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResponsesTool(pub Json);

pub fn transform_responses_request(
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    provider: ProviderId,
    model_info: Option<&Object>,
) -> Result<Object, ResponseInputError> {
    let tools = match params.get("tools") {
        None | Some(Json::Null) => None,
        Some(Json::List(entries)) => Some(
            entries
                .iter()
                .map(|entry| ResponsesTool(entry.clone()))
                .collect(),
        ),
        Some(other) => {
            return Err(ResponseInputError::ToolsNotAList {
                found: tools::source_type_name(other),
            });
        }
    };
    let (prepared_input, sanitized_tools) =
        prepared_input_and_tools(model, input, tools, provider, model_info)?;
    for key in ["model", "input"] {
        if params.contains_key(key) {
            return Err(ResponseInputError::DuplicateBodyKey { key });
        }
    }
    let mut body = params.clone();
    body.insert("model".to_owned(), Json::Text(model.to_owned()));
    body.insert("input".to_owned(), input_to_json(&prepared_input));
    if let Some(sanitized) = sanitized_tools {
        body.insert(
            "tools".to_owned(),
            Json::List(sanitized.into_iter().map(|tool| tool.0).collect()),
        );
    }
    Ok(body)
}

fn input_to_json(input: &ResponsesInput) -> Json {
    match input {
        ResponsesInput::Text(text) => Json::Text(text.clone()),
        ResponsesInput::Items(items) => Json::List(
            items
                .iter()
                .map(|item| Json::Object(item.0.clone()))
                .collect(),
        ),
    }
}
