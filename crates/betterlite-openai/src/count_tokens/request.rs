use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::{DefaultOnNull, serde_as};

use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::messages::InputItem;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<InputItem>),
}

#[derive(Debug, Serialize)]
pub struct CountRequestBody<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
}

#[serde_as]
#[derive(Deserialize)]
struct ChatFunction {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    name: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    description: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    parameters: Map<String, Value>,
}

pub const fn validate_request(model: &str, input: &ResponsesInput) -> Result<(), CountTokensError> {
    if model.is_empty() {
        return Err(CountTokensError::ModelRequired);
    }
    let empty = match input {
        ResponsesInput::Text(text) => text.is_empty(),
        ResponsesInput::Items(items) => items.is_empty(),
    };
    if empty {
        return Err(CountTokensError::InputRequired);
    }
    Ok(())
}

fn flatten_function(function: &Value) -> Result<Value, CountTokensError> {
    let Value::Object(fields) = function else {
        return Err(CountTokensError::ToolNotObject);
    };
    let parsed: ChatFunction = serde_json::from_value(function.clone())
        .map_err(|source| CountTokensError::Tool { source })?;
    let mut item = Map::new();
    item.insert(String::from("type"), Value::from("function"));
    item.insert(String::from("name"), Value::from(parsed.name));
    item.insert(String::from("description"), Value::from(parsed.description));
    item.insert(String::from("parameters"), Value::Object(parsed.parameters));
    if let Some(strict) = fields.get("strict") {
        item.insert(String::from("strict"), strict.clone());
    }
    Ok(Value::Object(item))
}

pub fn flatten_tools(tools: &[Map<String, Value>]) -> Result<Vec<Value>, CountTokensError> {
    tools
        .iter()
        .map(|tool| match (tool.get("type"), tool.get("function")) {
            (Some(Value::String(kind)), Some(function)) if kind == "function" => {
                flatten_function(function)
            }
            _ => Ok(Value::Object(tool.clone())),
        })
        .collect()
}

pub fn required_headers(api_key: &SecretString) -> Result<HeaderMap, CountTokensError> {
    let mut authorization = HeaderValue::try_from(format!("Bearer {}", api_key.expose_secret()))
        .map_err(|_invalid| CountTokensError::InvalidApiKey)?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(AUTHORIZATION, authorization);
    Ok(headers)
}
