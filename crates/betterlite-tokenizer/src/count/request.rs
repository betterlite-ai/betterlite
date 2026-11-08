use std::borrow::Cow;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::engine::Engine;
use crate::engine::selection::TokenizerType;

#[derive(Debug, Clone)]
pub enum SelectedTokenizer {
    OpenAi,
    HuggingFace(Engine),
}

impl SelectedTokenizer {
    #[must_use]
    pub fn from_selection(engine: Engine, kind: TokenizerType) -> Self {
        match kind {
            TokenizerType::OpenAi => Self::OpenAi,
            TokenizerType::HuggingFace => Self::HuggingFace(engine),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TextInput {
    One(String),
    Many(Vec<String>),
}

impl TextInput {
    #[must_use]
    pub fn joined(&self) -> Cow<'_, str> {
        match self {
            Self::One(text) => Cow::Borrowed(text.as_str()),
            Self::Many(parts) => Cow::Owned(parts.concat()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text(String),
    Block(ContentPart),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub fields: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Option<MessageContent>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub function_call: Option<FunctionCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub search_results: Option<Json>,
    // LiteLLM counts a string under any other key too, so those keys keep their JSON here
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallFunction {
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCall {
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Named(NamedToolChoice),
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedToolChoice {
    pub function: ToolFunctionName,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolFunctionName {
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct TokenRequest<'request> {
    pub model: &'request str,
    pub custom_tokenizer: Option<&'request SelectedTokenizer>,
    pub text: Option<&'request TextInput>,
    pub messages: Option<&'request [Message]>,
    pub count_response_tokens: bool,
    pub tools: Option<&'request [Json]>,
    pub tool_choice: Option<&'request ToolChoice>,
}
