use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

use super::error::OllamaError;

pub const ARGUMENTS_FIELD: &str = "message.tool_calls.function.arguments";

// the tool-call shapes shared by a reply and a stream chunk
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WireMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WireToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub index: Option<u32>,
    pub function: WireFunction,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WireFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<Json>,
    #[serde(default)]
    pub parameters: Option<Json>,
}

pub struct CallArguments {
    pub complete: bool,
    pub arguments: String,
}

impl WireFunction {
    // LiteLLM's Function plus its `len(function_args) > 0` guard: an object is dumped as JSON, a string passes
    // through, and null falls back to parameters
    pub fn call_arguments(&self) -> Result<CallArguments, OllamaError> {
        match &self.arguments {
            Some(value @ Json::Object(fields)) => Ok(CallArguments {
                complete: !fields.is_empty(),
                arguments: compact(value)?,
            }),
            Some(value @ Json::Text(text)) => Ok(CallArguments {
                complete: !text.is_empty() && is_function_call_complete(value),
                arguments: text.clone(),
            }),
            Some(Json::Null) | None => Ok(CallArguments {
                complete: false,
                arguments: self.parameters_text()?,
            }),
            Some(Json::List(_) | Json::Bool(_) | Json::Number(_)) => Err(OllamaError::Arguments {
                field: ARGUMENTS_FIELD,
            }),
        }
    }

    fn parameters_text(&self) -> Result<String, OllamaError> {
        match &self.parameters {
            Some(value @ Json::Object(_)) => compact(value),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => Ok(String::new()),
        }
    }
}

// LiteLLM's _is_function_call_complete: an object is complete, a string is complete when it parses as JSON
#[must_use]
pub fn is_function_call_complete(arguments: &Json) -> bool {
    match arguments {
        Json::Object(_) => true,
        Json::Text(text) => serde_json::from_str::<Json>(text).is_ok(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => false,
    }
}

fn compact(value: &Json) -> Result<String, OllamaError> {
    serde_json::to_string(value).map_err(|source| OllamaError::Shape { source })
}
