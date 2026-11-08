use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

pub type Payload = IndexMap<String, Json>;

// The dataclass fields whose value can be null; __dict__ keeps them, get_traces_and_spans_from_payload strips them.
pub const TRACE_OPTIONAL_FIELDS: [&str; 3] = ["thread_id", "input", "output"];
pub const SPAN_OPTIONAL_FIELDS: [&str; 5] = [
    "parent_span_id",
    "provider",
    "total_cost",
    "input",
    "output",
];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpanType {
    Llm,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TraceRecord {
    pub id: String,
    pub name: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    #[serde(default)]
    pub input: Option<Json>,
    #[serde(default)]
    pub output: Option<Json>,
    pub metadata: IndexMap<String, Json>,
    pub tags: Vec<Json>,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub project_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SpanRecord {
    pub id: String,
    pub trace_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub span_type: SpanType,
    pub model: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    #[serde(default)]
    pub input: Option<Json>,
    #[serde(default)]
    pub output: Option<Json>,
    pub metadata: IndexMap<String, Json>,
    pub tags: Vec<Json>,
    pub usage: IndexMap<String, Json>,
    pub project_name: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub total_cost: Option<Decimal>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("the Opik payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Opik payload does not match the callback record")]
    Decode(#[source] serde_json::Error),
}

impl TryFrom<&Payload> for TraceRecord {
    type Error = RecordError;

    fn try_from(payload: &Payload) -> Result<Self, Self::Error> {
        let text = serde_json::to_string(payload).map_err(RecordError::Encode)?;
        serde_json::from_str(&text).map_err(RecordError::Decode)
    }
}

impl TryFrom<&Payload> for SpanRecord {
    type Error = RecordError;

    fn try_from(payload: &Payload) -> Result<Self, Self::Error> {
        let text = serde_json::to_string(payload).map_err(RecordError::Encode)?;
        serde_json::from_str(&text).map_err(RecordError::Decode)
    }
}

// The sync send posts TracePayload.__dict__ and SpanPayload.__dict__, whose optional fields are present as null.
#[must_use]
pub fn wire_payload(payload: &Payload, optional_fields: &[&str]) -> Payload {
    let mut wire = payload.clone();
    for field in optional_fields {
        if !wire.contains_key(*field) {
            wire.insert((*field).to_owned(), Json::Null);
        }
    }
    wire
}
