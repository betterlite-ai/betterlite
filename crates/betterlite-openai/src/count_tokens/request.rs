use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::rust::double_option;

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
    pub tools: Option<Vec<ResponsesTool>>,
}

type PresentOrNull = Option<Value>;

#[derive(Deserialize)]
struct ChatFunction {
    #[serde(default = "default_name")]
    name: Value,
    #[serde(default = "default_description")]
    description: Value,
    #[serde(default = "default_parameters")]
    parameters: Value,
    #[serde(default, with = "double_option")]
    strict: Option<PresentOrNull>,
}

fn default_name() -> Value {
    Value::from("")
}

fn default_description() -> Value {
    Value::from("")
}

fn default_parameters() -> Value {
    Value::Object(Map::new())
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ResponsesFunction {
    r#type: &'static str,
    name: Value,
    description: Value,
    parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none", with = "double_option")]
    strict: Option<PresentOrNull>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponsesTool {
    Function(ResponsesFunction),
    Other(Map<String, Value>),
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

fn flatten_function(function: &Value) -> Result<ResponsesFunction, CountTokensError> {
    let Value::Object(fields) = function else {
        return Err(CountTokensError::ToolNotObject);
    };
    let parsed =
        ChatFunction::deserialize(fields).map_err(|source| CountTokensError::Tool { source })?;
    Ok(ResponsesFunction {
        r#type: "function",
        name: parsed.name,
        description: parsed.description,
        parameters: parsed.parameters,
        strict: parsed.strict,
    })
}

pub fn flatten_tools(tools: &[Map<String, Value>]) -> Result<Vec<ResponsesTool>, CountTokensError> {
    tools
        .iter()
        .map(|tool| match (tool.get("type"), tool.get("function")) {
            (Some(Value::String(kind)), Some(function)) if kind == "function" => {
                flatten_function(function).map(ResponsesTool::Function)
            }
            _ => Ok(ResponsesTool::Other(tool.clone())),
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
