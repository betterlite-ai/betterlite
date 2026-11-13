use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::{DefaultOnNull, serde_as};
use thiserror::Error;

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct Response {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
    #[serde(rename = "id")]
    pub _id: Option<String>,
    #[serde(rename = "created")]
    pub _created: Option<i64>,
    #[serde(rename = "model")]
    pub _model: Option<String>,
    #[serde(rename = "system_fingerprint")]
    pub _system_fingerprint: Option<String>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct Choice {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
pub(super) struct Delta {
    pub content: Option<String>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawCall")]
pub(super) enum ToolCall {
    Function(Tool),
    Custom,
}

#[derive(Debug, Error)]
#[error("a tool call needs a function object, or a custom object for type custom")]
pub(super) struct InvalidCall;

#[serde_as]
#[derive(Debug, Deserialize)]
struct RawCall {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    custom: Option<CustomPayload>,
    function: Option<MaybeFunction>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    index: i64,
}

#[derive(Debug, Deserialize)]
struct CustomPayload {
    #[serde(rename = "name")]
    _name: Option<String>,
    #[serde(rename = "input")]
    _input: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MaybeFunction {
    Valid(Function),
    Invalid(IgnoredAny),
}

impl TryFrom<RawCall> for ToolCall {
    type Error = InvalidCall;

    fn try_from(raw: RawCall) -> Result<Self, Self::Error> {
        if raw.custom.is_some() {
            return Ok(Self::Custom);
        }
        match raw.function {
            Some(MaybeFunction::Valid(function)) if raw.kind.as_deref() != Some("custom") => {
                Ok(Self::Function(Tool {
                    id: raw.id,
                    kind: "function".to_owned(),
                    function,
                    index: raw.index,
                }))
            }
            _ => Err(InvalidCall),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tool {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Function,
    pub index: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(from = "RawFunction")]
pub struct Function {
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
struct RawFunction {
    name: Option<String>,
    arguments: Option<Arguments>,
    parameters: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Arguments {
    Text(String),
    Object(Map<String, Value>),
}

impl From<RawFunction> for Function {
    fn from(raw: RawFunction) -> Self {
        let arguments = match (raw.arguments, raw.parameters) {
            (Some(Arguments::Text(text)), _) => text,
            (Some(Arguments::Object(object)), _) => Value::Object(object).to_string(),
            (None, Some(parameters @ Value::Object(_))) => parameters.to_string(),
            (None, _) => String::new(),
        };
        Self {
            name: raw.name,
            arguments,
        }
    }
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct Usage {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub prompt_tokens: i64,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub completion_tokens: i64,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub total_tokens: i64,
}
