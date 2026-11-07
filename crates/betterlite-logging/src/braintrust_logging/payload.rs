use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use self::metrics::{Metrics, metrics};
use self::output::output;

mod finish_reason;
mod metrics;
mod output;

const DEFAULT_SPAN_NAME: &str = "Chat Completion";

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("{0} must be {1}")]
    WrongType(&'static str, &'static str),
    #[error("litellm_params.metadata.{key} must be a string")]
    MetadataText { key: &'static str },
    #[error("response.usage.{name} must be an integer")]
    UsageToken { name: &'static str },
    #[error("the {field} time is not a JSON number")]
    Number {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {key} tag value is not JSON")]
    Tag {
        key: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Serialize)]
pub(super) struct Event<'call> {
    id: &'call Json,
    input: &'call Json,
    metadata: IndexMap<String, Json>,
    span_attributes: SpanAttributes,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_id: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_span_id: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_parents: Option<Vec<String>>,
    output: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<Metrics<'call>>,
}

#[derive(Serialize)]
struct SpanAttributes {
    name: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
pub(super) struct Events<'event, 'call> {
    pub(super) events: [&'event Event<'call>; 1],
}

pub(super) fn request_data<'call>(
    event: &SuccessEvent<'call>,
    default_tags: &[String],
    path: CallbackPath,
) -> Result<Event<'call>, PayloadError> {
    let metadata = event
        .metadata
        .filter(|value| !matches!(value, Json::Null))
        .map(|value| object(value, "litellm_params.metadata"))
        .transpose()?;
    let (logging, tags) = logging_object(metadata, event.standard_logging_object, default_tags)?;
    let span_name = metadata_text(metadata, "span_name")?
        .map_or_else(|| DEFAULT_SPAN_NAME.to_owned(), str::to_owned);
    let root = metadata
        .and_then(|fields| fields.get("root_span_id"))
        .is_none_or(|value| matches!(value, Json::Null));
    Ok(Event {
        id: event.call_id.unwrap_or(&Json::Null),
        input: event.messages.unwrap_or(&Json::Null),
        metadata: logging,
        span_attributes: SpanAttributes {
            name: span_name,
            kind: "llm",
        },
        tags: root.then_some(tags),
        span_id: metadata_text(metadata, "span_id")?.filter(|value| !value.is_empty()),
        root_span_id: metadata_text(metadata, "root_span_id")?.filter(|value| !value.is_empty()),
        span_parents: span_parents(metadata)?,
        output: output(event)?,
        metrics: metrics(event, path)?,
    })
}

fn logging_object(
    metadata: Option<&IndexMap<String, Json>>,
    standard_logging_object: Option<&Json>,
    default_tags: &[String],
) -> Result<(IndexMap<String, Json>, Vec<String>), PayloadError> {
    let standard = standard_logging_object.filter(|value| !matches!(value, Json::Null));
    let mut logging = standard
        .map(|value| object(value, "standard_logging_object"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let mut tags = Vec::new();
    let Some(fields) = metadata else {
        return Ok((logging, tags));
    };
    for (key, value) in fields {
        if default_tags.iter().any(|tag| tag == key) {
            tags.push(format!("{key}:{}", tag_text(key, value)?));
        }
        if let Json::Text(text) = value
            && !logging.contains_key(key)
        {
            logging.insert(key.clone(), Json::Text(text.clone()));
        }
    }
    Ok((logging, tags))
}

fn span_parents(
    metadata: Option<&IndexMap<String, Json>>,
) -> Result<Option<Vec<String>>, PayloadError> {
    let Some(parents) = metadata_text(metadata, "span_parents")? else {
        return Ok(None);
    };
    let ids: Vec<String> = parents
        .split(',')
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned)
        .collect();
    Ok((!ids.is_empty()).then_some(ids))
}

fn tag_text(key: &str, value: &Json) -> Result<String, PayloadError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Bool(flag) => Ok(flag.to_string()),
        Json::Null => Ok("null".to_owned()),
        Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).map_err(|source| PayloadError::Tag {
                key: key.to_owned(),
                source,
            })
        }
    }
}

pub(crate) const fn object<'json>(
    value: &'json Json,
    field: &'static str,
) -> Result<&'json IndexMap<String, Json>, PayloadError> {
    if let Json::Object(fields) = value {
        return Ok(fields);
    }
    Err(PayloadError::WrongType(field, "a JSON object"))
}

pub(crate) fn metadata_text<'json>(
    metadata: Option<&'json IndexMap<String, Json>>,
    key: &'static str,
) -> Result<Option<&'json str>, PayloadError> {
    let Some(value) = metadata.and_then(|fields| fields.get(key)) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text)),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(PayloadError::MetadataText { key })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackPath {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy)]
pub enum ResponseObject<'response> {
    Model(&'response Json),
    TextCompletion(&'response Json),
    Embedding(&'response Json),
    Image(&'response Json),
}

#[derive(Debug, Clone, Copy)]
pub struct SuccessEvent<'call> {
    pub call_id: Option<&'call Json>,
    pub messages: Option<&'call Json>,
    pub standard_logging_object: Option<&'call Json>,
    pub metadata: Option<&'call Json>,
    pub response_cost: Option<&'call Json>,
    pub call_type: Option<&'call str>,
    pub response: Option<ResponseObject<'call>>,
    pub api_call_start_time: Option<DateTime<Utc>>,
    pub completion_start_time: Option<DateTime<Utc>>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}
