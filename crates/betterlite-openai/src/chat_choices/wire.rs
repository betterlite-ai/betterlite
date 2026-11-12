use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::{DefaultOnNull, serde_as};

// a provider reply choice, of which _transform_choices reads the fields below
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplyChoice {
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub message: ReplyMessage,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplyMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: ProviderField,
    #[serde(default)]
    pub reasoning: ProviderField,
    #[serde(default)]
    pub tool_calls: Option<Vec<ReplyToolCall>>,
}

// _extract_reasoning_content tests key presence, so an explicit null differs from a missing key
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ProviderField {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null,
    Text(String),
}

// ChatCompletionMessageToolCall(**tool_call) keeps the keys it does not name, such as index
#[derive(Debug, Clone, Deserialize)]
pub struct ReplyToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub function: Option<ReplyFunction>,
    #[serde(default)]
    pub custom: Option<ReplyCustom>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyCustom {
    pub name: String,
    pub input: String,
}

// LiteLLM's Choices, reduced to the fields _transform_choices can set
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransformedChoice {
    pub index: i64,
    pub finish_reason: String,
    pub message: ChoiceMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ChoiceToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CallFunction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomPayload>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallFunction {
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomPayload {
    pub name: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamReply {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: Option<String>,
    pub choices: Vec<StreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamChoice {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub index: i64,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub delta: StreamDelta,
    #[serde(default)]
    pub logprobs: Option<ChoiceLogprobs>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceLogprobs {
    #[serde(default)]
    pub content: Option<Vec<TokenLogprob>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenLogprob {
    pub token: String,
    #[serde(default)]
    pub bytes: Option<Vec<i64>>,
    pub logprob: f64,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub top_logprobs: Vec<TopLogprob>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopLogprob {
    pub token: String,
    #[serde(default)]
    pub bytes: Option<Vec<i64>>,
    pub logprob: f64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub function_call: Option<DeltaFunctionCall>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    #[serde(default)]
    pub audio: Option<DeltaAudio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing)]
    pub reasoning: ProviderField,
    #[serde(flatten)]
    pub passthrough: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaFunctionCall {
    pub arguments: String,
    #[serde(default)]
    pub name: Option<String>,
}

// Delta.__init__ numbers the calls that lack an index and types the function ones, after parsing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaToolCall {
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<DeltaFunction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<DeltaCustom>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawDeltaFunction")]
pub struct DeltaFunction {
    pub name: Option<String>,
    pub arguments: String,
}

// Function.__init__ accepts arguments as text, a dict or None, and falls back to a dict `parameters` for None
#[derive(Deserialize)]
struct RawDeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: FunctionArguments,
    #[serde(default)]
    parameters: Option<Value>,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
pub(crate) enum FunctionArguments {
    Text(String),
    Object(Map<String, Value>),
    #[default]
    Null,
}

impl FunctionArguments {
    pub(crate) fn into_text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::Object(fields) => Some(Value::Object(fields).to_string()),
            Self::Null => None,
        }
    }
}

impl From<RawDeltaFunction> for DeltaFunction {
    fn from(raw: RawDeltaFunction) -> Self {
        let arguments = raw
            .arguments
            .into_text()
            .or_else(|| match raw.parameters {
                Some(parameters @ Value::Object(_)) => Some(parameters.to_string()),
                _ => None,
            })
            .unwrap_or_default();
        Self {
            name: raw.name,
            arguments,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaCustom {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaAudio {
    pub data: String,
    pub expires_at: i64,
    pub transcript: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
