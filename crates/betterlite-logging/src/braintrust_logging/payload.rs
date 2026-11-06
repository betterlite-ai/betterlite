use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use thiserror::Error;

use self::metrics::metrics;
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
}

pub(super) fn request_data(
    event: &SuccessEvent<'_>,
    default_tags: &[String],
    path: CallbackPath,
) -> Result<Json, PayloadError> {
    let metadata = event
        .metadata
        .filter(|value| !matches!(value, Json::Null))
        .map(|value| object(value, "litellm_params.metadata"))
        .transpose()?;
    let (logger_metadata, tags) =
        logging_object(metadata, event.standard_logging_object, default_tags)?;
    let span_name = metadata_text(metadata, "span_name")?
        .map_or_else(|| DEFAULT_SPAN_NAME.to_owned(), str::to_owned);
    let mut data = IndexMap::new();
    data.insert(
        "id".to_owned(),
        event.call_id.cloned().unwrap_or(Json::Null),
    );
    data.insert(
        "input".to_owned(),
        event.messages.cloned().unwrap_or(Json::Null),
    );
    data.insert("metadata".to_owned(), Json::Object(logger_metadata));
    data.insert(
        "span_attributes".to_owned(),
        Json::Object(IndexMap::from([
            ("name".to_owned(), Json::Text(span_name)),
            ("type".to_owned(), Json::Text("llm".to_owned())),
        ])),
    );
    if metadata
        .and_then(|fields| fields.get("root_span_id"))
        .is_none_or(|value| matches!(value, Json::Null))
    {
        data.insert("tags".to_owned(), Json::List(tags));
    }
    span_keys(&mut data, metadata)?;
    data.insert("output".to_owned(), output(event)?);
    if let Some(metrics) = metrics(event, path)? {
        data.insert("metrics".to_owned(), metrics);
    }
    Ok(Json::Object(data))
}

fn logging_object(
    metadata: Option<&IndexMap<String, Json>>,
    standard_logging_object: Option<&Json>,
    default_tags: &[String],
) -> Result<(IndexMap<String, Json>, Vec<Json>), PayloadError> {
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
            tags.push(Json::Text(format!("{key}:{}", tag_text(value))));
        }
        if let Json::Text(text) = value
            && !logging.contains_key(key)
        {
            logging.insert(key.clone(), Json::Text(text.clone()));
        }
    }
    Ok((logging, tags))
}

fn span_keys(
    data: &mut IndexMap<String, Json>,
    metadata: Option<&IndexMap<String, Json>>,
) -> Result<(), PayloadError> {
    for key in ["span_id", "root_span_id"] {
        if let Some(value) = metadata_text(metadata, key)?
            && !value.is_empty()
        {
            data.insert(key.to_owned(), Json::Text(value.to_owned()));
        }
    }
    let Some(parents) = metadata_text(metadata, "span_parents")? else {
        return Ok(());
    };
    let ids: Vec<Json> = parents
        .split(',')
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
        .map(|parent| Json::Text(parent.to_owned()))
        .collect();
    if !ids.is_empty() {
        data.insert("span_parents".to_owned(), Json::List(ids));
    }
    Ok(())
}

fn tag_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Number(number) => number.to_string(),
        Json::Bool(flag) => flag.to_string(),
        Json::Null => "null".to_owned(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
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
