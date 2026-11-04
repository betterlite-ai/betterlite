use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::value::RawValue;
use serde_with::{Map, serde_as};

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Index {
    Number(serde_json::Number),
    Boolean(bool),
    Text(String),
}

#[derive(Deserialize)]
pub(super) struct Record {
    pub index: Option<Index>,
    pub text: Option<String>,
    pub is_finished: Option<bool>,
    pub finish_reason: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub event: Option<String>,
    pub delta: Option<Box<RawValue>>,
    pub data: Option<Data>,
    pub citations: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
pub(super) struct Data {
    pub delta: Option<EventDelta>,
}

#[derive(Deserialize)]
pub(super) struct EventDelta {
    pub message: Option<Message>,
    pub finish_reason: Option<String>,
    pub usage: Option<UsageData>,
}

#[serde_as]
#[derive(Deserialize)]
pub(super) struct UsageData {
    pub tokens: Option<Tokens>,
    // Unmodeled keys still make the provider usage object nonempty.
    #[serde(flatten)]
    #[serde_as(as = "Map<_, _>")]
    pub other: Vec<(String, IgnoredAny)>,
}

#[derive(Deserialize)]
pub(super) struct Tokens {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct Delta {
    pub message: Option<Message>,
    pub tool_calls: Option<Vec<Tool>>,
}

#[derive(Deserialize)]
pub(super) struct Message {
    pub content: Option<Content>,
    pub tool_plan: Option<String>,
    pub citations: Option<Citation>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Content {
    Text(String),
    Object { text: Option<String> },
}

#[derive(Deserialize)]
pub(super) struct Tool {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
pub struct Citation {
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub text: Option<String>,
    pub sources: Option<Vec<Box<RawValue>>>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(flatten, skip_serializing)]
    #[serde_as(deserialize_as = "Map<_, _>")]
    pub other: Vec<(String, IgnoredAny)>,
}

#[derive(Debug, Serialize)]
pub struct ProviderFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<Box<RawValue>>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: Function,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Function {
    pub name: String,
    pub arguments: String,
}
