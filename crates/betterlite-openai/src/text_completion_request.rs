use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;
use thiserror::Error;

use crate::chat::{
    FunctionDefinition, FunctionName, ResponseFormat, Stop, StreamOptions, Tool, ToolChoice,
};
use crate::text_completion_policy::SUPPORTED_PARAMS;

// main.py's text_completion reads these off the request and never forwards them to a provider
const CONTROL_PARAMS: [&str; 16] = [
    "api_base",
    "api_key",
    "api_version",
    "custom_llm_provider",
    "deployment_id",
    "engine",
    "fallbacks",
    "litellm_params",
    "metadata",
    "mock_delay",
    "mock_response",
    "mock_timeout",
    "mock_tool_calls",
    "model_list",
    "num_retries",
    "timeout",
];

#[derive(Debug, Error)]
pub enum CompletionRequestError {
    #[error("the request body is not a valid completion request: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite does not support the {name} completion parameter")]
    Unsupported { name: String },
    #[error("the completion request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// OpenAI's completion prompt: one string, a list of strings, token ids, or one token list per prompt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionPrompt {
    Text(String),
    Texts(Vec<String>),
    Tokens(Vec<i64>),
    TokenLists(Vec<Vec<i64>>),
    // a shape the declared prompt union reaches but main.py's dispatch refuses
    Unmapped(Value),
}

fn integers(items: &[Value]) -> Option<Vec<i64>> {
    items.iter().map(Value::as_i64).collect()
}

// main.py 7623-7692 checks the first element only, so an empty or mixed list is the Unmapped shape
fn classify(items: Vec<Value>) -> CompletionPrompt {
    let Some(first) = items.first() else {
        return CompletionPrompt::Unmapped(Value::Array(items));
    };
    if first.is_string() {
        return items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<String>>>()
            .map_or_else(
                || CompletionPrompt::Unmapped(Value::Array(items)),
                CompletionPrompt::Texts,
            );
    }
    if first.is_number() {
        return integers(&items).map_or_else(
            || CompletionPrompt::Unmapped(Value::Array(items)),
            CompletionPrompt::Tokens,
        );
    }
    if first.is_array() {
        return items
            .iter()
            .map(|item| item.as_array().and_then(|inner| integers(inner)))
            .collect::<Option<Vec<Vec<i64>>>>()
            .map_or_else(
                || CompletionPrompt::Unmapped(Value::Array(items)),
                CompletionPrompt::TokenLists,
            );
    }
    CompletionPrompt::Unmapped(Value::Array(items))
}

impl CompletionPrompt {
    // main.py builds one user message per string, or one user message holding the token prompt
    #[must_use]
    pub fn messages(&self) -> Option<Vec<Value>> {
        match self {
            Self::Texts(texts) if !texts.is_empty() => Some(
                texts
                    .iter()
                    .map(|text| user_message(Value::String(text.clone())))
                    .collect(),
            ),
            Self::Text(_) | Self::Tokens(_) | Self::TokenLists(_) => {
                Some(vec![user_message(self.to_value())])
            }
            Self::Texts(_) | Self::Unmapped(_) => None,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Text(text) => Value::String(text.clone()),
            Self::Texts(texts) => Value::Array(texts.iter().cloned().map(Value::String).collect()),
            Self::Tokens(tokens) => {
                Value::Array(tokens.iter().map(|id| Value::from(*id)).collect())
            }
            Self::TokenLists(lists) => Value::Array(
                lists
                    .iter()
                    .map(|ids| Value::Array(ids.iter().map(|id| Value::from(*id)).collect()))
                    .collect(),
            ),
            Self::Unmapped(value) => value.clone(),
        }
    }
}

impl Serialize for CompletionPrompt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CompletionPrompt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Value::deserialize(deserializer)? {
            Value::String(text) => Self::Text(text),
            Value::Array(items) => classify(items),
            other => Self::Unmapped(other),
        })
    }
}

fn user_message(content: Value) -> Value {
    Value::Object(Map::from_iter([
        (String::from("role"), Value::String(String::from("user"))),
        (String::from("content"), content),
    ]))
}

// The completion body the proxy accepts; params betterlite does not model stay in passthrough until parse refuses them
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CompletionRequest {
    pub model: Option<String>,
    pub prompt: CompletionPrompt,
    pub stream: Option<bool>,
    pub stream_options: Option<StreamOptions>,
    pub max_tokens: Option<i64>,
    pub temperature: Option<serde_json::Number>,
    pub top_p: Option<serde_json::Number>,
    pub n: Option<i64>,
    pub stop: Option<Stop>,
    pub presence_penalty: Option<serde_json::Number>,
    pub frequency_penalty: Option<serde_json::Number>,
    pub logit_bias: Option<BTreeMap<String, i64>>,
    pub logprobs: Option<i64>,
    pub top_logprobs: Option<i64>,
    pub user: Option<String>,
    pub seed: Option<i64>,
    pub response_format: Option<ResponseFormat>,
    pub functions: Option<Vec<FunctionDefinition>>,
    pub function_call: Option<FunctionCallChoice>,
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
    pub max_retries: Option<i64>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum FunctionCallChoice {
    Mode(String),
    Named(FunctionName),
}

impl CompletionRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, CompletionRequestError> {
        let request: Self = serde_json::from_slice(bytes)
            .map_err(|source| CompletionRequestError::Body { source })?;
        request.check_params()?;
        Ok(request)
    }

    // every key the provider request would carry must be one the text config supports; controls never reach it
    fn check_params(&self) -> Result<(), CompletionRequestError> {
        for name in self.passthrough.keys() {
            if CONTROL_PARAMS.contains(&name.as_str()) || SUPPORTED_PARAMS.contains(&name.as_str())
            {
                continue;
            }
            return Err(CompletionRequestError::Unsupported { name: name.clone() });
        }
        Ok(())
    }

    // the transform overlays these onto model/prompt; stream is a control here because a stream request never reaches the text endpoint
    pub fn optional_params(&self) -> Result<Map<String, Value>, CompletionRequestError> {
        let Value::Object(mut params) = serde_json::to_value(self)
            .map_err(|source| CompletionRequestError::Encode { source })?
        else {
            return Err(CompletionRequestError::Encode {
                source: serde_json::Error::io(std::io::Error::other(
                    "the completion request did not serialize as an object",
                )),
            });
        };
        params.remove("model");
        params.remove("prompt");
        params.remove("stream");
        params.retain(|name, _| SUPPORTED_PARAMS.contains(&name.as_str()));
        Ok(params)
    }
}

mod tests;
