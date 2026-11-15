use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;
use thiserror::Error;

use crate::chat::{
    FunctionDefinition, FunctionName, ResponseFormat, Stop, StreamOptions, Tool, ToolChoice,
};
use crate::text_completion_policy::SUPPORTED_PARAMS;

// main.py's text_completion reads these off the request and never forwards them to a provider
const CONTROL_PARAMS: [&str; 10] = [
    "api_base",
    "api_version",
    "engine",
    "litellm_params",
    "metadata",
    "mock_delay",
    "mock_response",
    "mock_timeout",
    "mock_tool_calls",
    "num_retries",
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "Value", into = "Value")]
pub enum CompletionPrompt {
    Text(String),
    Texts(Vec<String>),
    Tokens(Vec<i64>),
    TokenLists(Vec<Vec<i64>>),
    // a shape main.py's dispatch or its file-id pass refuses before any provider call
    Unmapped(Value),
    // a token-branch content that is not token ids, which the provider transform cannot read
    Unreadable(Value),
}

// How LiteLLM fails a prompt it cannot send: the APIConnectionError shape or the InternalServerError shape
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRefusal {
    Unmapped,
    Unreadable,
}

// main.py 7623-7692 dispatches on the first element only, so a later element it cannot convert decides the failure shape
fn classify(items: Vec<Value>) -> CompletionPrompt {
    let Some(first) = items.first() else {
        return CompletionPrompt::Unmapped(Value::Array(items));
    };
    if first.is_string() {
        return string_first(items);
    }
    // Python's int test accepts no bool or float, so only a JSON integer starts the token branch
    if matches!(first, Value::Number(number) if number.is_i64() || number.is_u64()) {
        return items
            .iter()
            .map(Value::as_i64)
            .collect::<Option<Vec<i64>>>()
            .map_or_else(
                || CompletionPrompt::Unreadable(Value::Array(items)),
                CompletionPrompt::Tokens,
            );
    }
    if first.is_array() {
        return items
            .iter()
            .map(|item| {
                item.as_array()
                    .and_then(|inner| inner.iter().map(Value::as_i64).collect())
            })
            .collect::<Option<Vec<Vec<i64>>>>()
            .map_or_else(
                || CompletionPrompt::Unreadable(Value::Array(items)),
                CompletionPrompt::TokenLists,
            );
    }
    CompletionPrompt::Unmapped(Value::Array(items))
}

// the file-id pass walks every message before any transform, so a truthy number or bool fails the whole prompt first
fn string_first(items: Vec<Value>) -> CompletionPrompt {
    let scalars = items.iter().any(|item| {
        matches!(item, Value::Bool(true))
            || matches!(item, Value::Number(number) if number.as_f64() != Some(0.0))
    });
    if scalars {
        return CompletionPrompt::Unmapped(Value::Array(items));
    }
    // a later non-empty list is one convert_content_list_to_str cannot read; every other later non-string reads as ""
    if items
        .iter()
        .any(|item| matches!(item, Value::Array(content) if !content.is_empty()))
    {
        return CompletionPrompt::Unreadable(Value::Array(items));
    }
    CompletionPrompt::Texts(
        items
            .iter()
            .map(|item| item.as_str().map_or_else(String::new, str::to_owned))
            .collect(),
    )
}

impl CompletionPrompt {
    // main.py builds one user message per string, or one user message holding the token prompt
    #[must_use]
    pub fn messages(&self) -> Option<Vec<Value>> {
        if let Self::Texts(texts) = self {
            return (!texts.is_empty()).then(|| {
                texts
                    .iter()
                    .map(|text| user_message(Value::from(text.as_str())))
                    .collect()
            });
        }
        (!matches!(self, Self::Unmapped(_) | Self::Unreadable(_)))
            .then(|| vec![user_message(Value::from(self.clone()))])
    }

    #[must_use]
    pub const fn refusal(&self) -> Option<PromptRefusal> {
        match self {
            Self::Unmapped(_) => Some(PromptRefusal::Unmapped),
            Self::Unreadable(_) => Some(PromptRefusal::Unreadable),
            Self::Text(_) | Self::Texts(_) | Self::Tokens(_) | Self::TokenLists(_) => None,
        }
    }
}

impl From<Value> for CompletionPrompt {
    fn from(value: Value) -> Self {
        match value {
            Value::String(text) => Self::Text(text),
            Value::Array(items) => classify(items),
            other => Self::Unmapped(other),
        }
    }
}

impl From<CompletionPrompt> for Value {
    fn from(prompt: CompletionPrompt) -> Self {
        match prompt {
            CompletionPrompt::Text(text) => Self::String(text),
            CompletionPrompt::Texts(texts) => Self::from(texts),
            CompletionPrompt::Tokens(tokens) => Self::from(tokens),
            CompletionPrompt::TokenLists(lists) => Self::from(lists),
            CompletionPrompt::Unmapped(value) | CompletionPrompt::Unreadable(value) => value,
        }
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
    // the completion call's own deadline, which replaces the deployment's timeout
    pub timeout: Option<f64>,
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
        let mut request: Self = serde_json::from_slice(bytes)
            .map_err(|source| CompletionRequestError::Body { source })?;
        // a client api_key is never used; taking it out here keeps the plaintext out of every log and Debug print
        request.passthrough.retain(|name, _| name != "api_key");
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
