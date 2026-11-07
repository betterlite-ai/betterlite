use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;

pub type Payload = IndexMap<String, Json>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanType {
    Llm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceRecord {
    pub id: String,
    pub name: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub input: Json,
    pub output: Json,
    pub metadata: IndexMap<String, Json>,
    pub tags: Vec<Json>,
    pub thread_id: Option<String>,
    pub project_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanRecord {
    pub id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub span_type: SpanType,
    pub model: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub input: Json,
    pub output: Json,
    pub metadata: IndexMap<String, Json>,
    pub tags: Vec<Json>,
    pub usage: IndexMap<String, Json>,
    pub project_name: String,
    pub provider: Option<String>,
    pub total_cost: Option<Decimal>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("the Opik payload has no {field} field")]
    Missing { field: &'static str },
    #[error("the Opik payload field {field} is not {expected}")]
    Type {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the Opik payload timestamp {field} is not RFC 3339")]
    Timestamp {
        field: &'static str,
        #[source]
        source: chrono::ParseError,
    },
    #[error("the Opik payload span type {value:?} is not llm")]
    SpanType { value: String },
    #[error("the Opik payload total_cost is not a decimal number")]
    Cost,
}

pub fn trace_record(payload: &Payload) -> Result<TraceRecord, RecordError> {
    Ok(TraceRecord {
        id: string_field(payload, "id")?,
        name: string_field(payload, "name")?,
        start_time: timestamp_field(payload, "start_time")?,
        end_time: timestamp_field(payload, "end_time")?,
        input: any_field(payload, "input")?,
        output: any_field(payload, "output")?,
        metadata: object_field(payload, "metadata")?,
        tags: list_field(payload, "tags")?,
        thread_id: optional_string_field(payload, "thread_id")?,
        project_name: string_field(payload, "project_name")?,
    })
}

pub fn span_record(payload: &Payload) -> Result<SpanRecord, RecordError> {
    Ok(SpanRecord {
        id: string_field(payload, "id")?,
        trace_id: string_field(payload, "trace_id")?,
        parent_span_id: optional_string_field(payload, "parent_span_id")?,
        name: string_field(payload, "name")?,
        span_type: span_type_field(payload)?,
        model: string_field(payload, "model")?,
        start_time: timestamp_field(payload, "start_time")?,
        end_time: timestamp_field(payload, "end_time")?,
        input: any_field(payload, "input")?,
        output: any_field(payload, "output")?,
        metadata: object_field(payload, "metadata")?,
        tags: list_field(payload, "tags")?,
        usage: object_field(payload, "usage")?,
        project_name: string_field(payload, "project_name")?,
        provider: optional_string_field(payload, "provider")?,
        total_cost: cost_field(payload)?,
    })
}

// The sync send posts TracePayload.__dict__ and SpanPayload.__dict__, whose optional fields are present as null.
#[must_use]
pub fn trace_wire(payload: &Payload) -> Payload {
    let mut wire = payload.clone();
    if !wire.contains_key("thread_id") {
        wire.insert("thread_id".to_owned(), Json::Null);
    }
    wire
}

#[must_use]
pub fn span_wire(payload: &Payload) -> Payload {
    let mut wire = payload.clone();
    for field in ["parent_span_id", "provider", "total_cost"] {
        if !wire.contains_key(field) {
            wire.insert(field.to_owned(), Json::Null);
        }
    }
    wire
}

fn string_field(payload: &Payload, field: &'static str) -> Result<String, RecordError> {
    match payload.get(field) {
        Some(Json::Text(value)) => Ok(value.clone()),
        Some(_) => Err(RecordError::Type {
            field,
            expected: "a string",
        }),
        None => Err(RecordError::Missing { field }),
    }
}

fn optional_string_field(
    payload: &Payload,
    field: &'static str,
) -> Result<Option<String>, RecordError> {
    match payload.get(field) {
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(_) => Err(RecordError::Type {
            field,
            expected: "a string",
        }),
    }
}

fn object_field(
    payload: &Payload,
    field: &'static str,
) -> Result<IndexMap<String, Json>, RecordError> {
    match payload.get(field) {
        Some(Json::Object(value)) => Ok(value.clone()),
        Some(_) => Err(RecordError::Type {
            field,
            expected: "an object",
        }),
        None => Err(RecordError::Missing { field }),
    }
}

fn list_field(payload: &Payload, field: &'static str) -> Result<Vec<Json>, RecordError> {
    match payload.get(field) {
        Some(Json::List(value)) => Ok(value.clone()),
        Some(_) => Err(RecordError::Type {
            field,
            expected: "a list",
        }),
        None => Err(RecordError::Missing { field }),
    }
}

fn any_field(payload: &Payload, field: &'static str) -> Result<Json, RecordError> {
    payload
        .get(field)
        .cloned()
        .ok_or(RecordError::Missing { field })
}

fn timestamp_field(payload: &Payload, field: &'static str) -> Result<DateTime<Utc>, RecordError> {
    match DateTime::parse_from_rfc3339(&string_field(payload, field)?) {
        Ok(parsed) => Ok(parsed.with_timezone(&Utc)),
        Err(source) => Err(RecordError::Timestamp { field, source }),
    }
}

fn span_type_field(payload: &Payload) -> Result<SpanType, RecordError> {
    match string_field(payload, "type")?.as_str() {
        "llm" => Ok(SpanType::Llm),
        value => Err(RecordError::SpanType {
            value: value.to_owned(),
        }),
    }
}

fn cost_field(payload: &Payload) -> Result<Option<Decimal>, RecordError> {
    match payload.get("total_cost") {
        Some(Json::Number(number)) => Decimal::from_str(&number.to_string())
            .map(Some)
            .map_err(|_source| RecordError::Cost),
        Some(Json::Null) | None => Ok(None),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(RecordError::Cost)
        }
    }
}
