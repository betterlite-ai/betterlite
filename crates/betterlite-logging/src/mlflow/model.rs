use std::collections::BTreeMap;

use serde::Serialize;

pub type Timestamp = chrono::DateTime<chrono::Utc>;

// The SDK adapter supplies typed payloads and Python truthiness without erasing them into JSON.
pub trait Payload: Clone + Serialize {
    fn is_truthy(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SpanKind {
    Llm,
    Embedding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Ok,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Span {
    pub request_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExceptionInfo {
    #[serde(rename = "exception.type")]
    pub exception_type: String,
    #[serde(rename = "exception.message")]
    pub message: String,
    #[serde(rename = "exception.stacktrace")]
    pub stacktrace: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "name", content = "attributes")]
pub enum Event {
    #[serde(rename = "streaming_chunk")]
    StreamingChunk { delta: String },
    #[serde(rename = "exception")]
    Exception(ExceptionInfo),
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Response<Data> {
    pub choices: Vec<Choice<Data>>,
    pub data: Option<Data>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Choice<Data> {
    // The adapter passes message.model_dump(exclude_none=True) and str(delta.model_dump).
    pub message: Option<Data>,
    pub delta_method: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Inputs<Data> {
    pub messages: Option<Vec<Data>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Data>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<Vec<Data>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Data>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction: Option<Data>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct OptionalParams<Data> {
    pub functions: Option<Vec<Data>>,
    pub tools: Option<Vec<Data>>,
    pub stream: Option<bool>,
    pub tool_choice: Option<Data>,
    pub user: Option<String>,
    pub remaining: BTreeMap<String, Data>,
}
