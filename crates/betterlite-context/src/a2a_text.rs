use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum A2aTextError {
    #[error("A2A message must be an object")]
    MessageNotObject,
    #[error("A2A message parts must be iterable")]
    PartsNotIterable,
    #[error("A2A message part must be an object")]
    PartNotObject,
    #[error("A2A text part text must be a string")]
    TextNotString,
    #[error("A2A artifacts must have a length")]
    ArtifactsNotSized,
    #[error("A2A artifacts object has no first item")]
    ArtifactsNotIndexed,
}

// A key set to null is still present, so a plain Option would lose it
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum Field<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Present(T),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Shape {
    Null,
    Flag(bool),
    Number(f64),
    Text(String),
    List(Vec<IgnoredAny>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RecordOr<T> {
    Record(T),
    Other(Shape),
}

pub type RecordValue = RecordOr<A2aRecord>;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ListValue {
    List(Vec<RecordValue>),
    Other(Shape),
}

#[derive(Debug, Default, Deserialize)]
pub struct A2aRecord {
    #[serde(default)]
    pub kind: Field<Shape>,
    #[serde(default)]
    pub text: Field<Shape>,
    #[serde(default)]
    pub parts: Field<ListValue>,
    #[serde(flatten)]
    pub other_keys: BTreeMap<String, Shape>,
}

#[derive(Debug, Deserialize)]
pub struct A2aResponse {
    pub result: Option<RecordOr<Box<A2aResult>>>,
}

#[derive(Debug, Deserialize)]
pub struct A2aResult {
    #[serde(default)]
    pub parts: Field<ListValue>,
    pub message: Option<RecordValue>,
    pub artifact: Option<RecordValue>,
    pub status: Option<RecordOr<A2aStatus>>,
    pub artifacts: Option<ListValue>,
}

#[derive(Debug, Deserialize)]
pub struct A2aStatus {
    pub message: Option<RecordValue>,
}

fn shape_is_truthy(shape: &Shape) -> bool {
    match shape {
        Shape::Null => false,
        Shape::Flag(flag) => *flag,
        Shape::Number(number) => *number != 0.0_f64,
        Shape::Text(text) => !text.is_empty(),
        Shape::List(items) => !items.is_empty(),
        Shape::Object(keys) => !keys.is_empty(),
    }
}

impl RecordValue {
    fn is_truthy(&self) -> bool {
        if let Self::Record(record) = self {
            return matches!(record.kind, Field::Present(_))
                || matches!(record.text, Field::Present(_))
                || matches!(record.parts, Field::Present(_))
                || !record.other_keys.is_empty();
        }
        let Self::Other(shape) = self else {
            return false;
        };
        shape_is_truthy(shape)
    }
}

pub fn extract_text_from_a2a_message(
    message: Option<&RecordValue>,
    depth: usize,
    max_depth: usize,
) -> Result<String, A2aTextError> {
    match message {
        None => Ok(String::new()),
        Some(_) if depth >= max_depth => Ok(String::new()),
        Some(RecordOr::Other(_)) => Err(A2aTextError::MessageNotObject),
        Some(RecordOr::Record(record)) => parts_text(&record.parts, depth, max_depth),
    }
}

fn parts_text(
    parts: &Field<ListValue>,
    depth: usize,
    max_depth: usize,
) -> Result<String, A2aTextError> {
    let items = match parts {
        _ if depth >= max_depth => return Ok(String::new()),
        Field::Present(ListValue::List(items)) => items,
        Field::Present(ListValue::Other(Shape::Null | Shape::Flag(_) | Shape::Number(_))) => {
            return Err(A2aTextError::PartsNotIterable);
        }
        // Python iterates a string or object, so only an empty one gets past the first part
        Field::Present(ListValue::Other(shape)) if shape_is_truthy(shape) => {
            return Err(A2aTextError::PartNotObject);
        }
        Field::Absent | Field::Present(ListValue::Other(_)) => return Ok(String::new()),
    };
    let mut texts = Vec::new();
    for item in items {
        let RecordOr::Record(part) = item else {
            return Err(A2aTextError::PartNotObject);
        };
        match (&part.kind, &part.parts) {
            // LiteLLM's join raises on a non-string only after every part was read
            (Field::Present(Shape::Text(kind)), _) if kind == "text" => {
                texts.push(match &part.text {
                    Field::Absent => Ok(String::new()),
                    Field::Present(Shape::Text(text)) => Ok(text.clone()),
                    Field::Present(_) => Err(A2aTextError::TextNotString),
                });
            }
            (_, Field::Absent) => {}
            (_, nested_parts) => {
                let nested = parts_text(nested_parts, depth.saturating_add(1), max_depth)?;
                if !nested.is_empty() {
                    texts.push(Ok(nested));
                }
            }
        }
    }
    Ok(texts.into_iter().collect::<Result<Vec<_>, _>>()?.join(" "))
}

pub fn extract_text_from_a2a_response(
    response: &A2aResponse,
    max_depth: usize,
) -> Result<String, A2aTextError> {
    let Some(RecordOr::Record(result)) = &response.result else {
        return Ok(String::new());
    };
    if matches!(result.parts, Field::Present(_)) {
        return parts_text(&result.parts, 0, max_depth);
    }
    if let Some(message) = result.message.as_ref().filter(|value| value.is_truthy()) {
        return extract_text_from_a2a_message(Some(message), 0, max_depth);
    }
    if let Some(artifact @ RecordOr::Record(_)) =
        result.artifact.as_ref().filter(|value| value.is_truthy())
    {
        return extract_text_from_a2a_message(Some(artifact), 0, max_depth);
    }
    if let Some(RecordOr::Record(status)) = &result.status
        && let Some(message) = status.message.as_ref().filter(|value| value.is_truthy())
    {
        return extract_text_from_a2a_message(Some(message), 0, max_depth);
    }
    match &result.artifacts {
        Some(ListValue::List(items)) => extract_text_from_a2a_message(items.first(), 0, max_depth),
        Some(ListValue::Other(shape)) if shape_is_truthy(shape) => match shape {
            Shape::Text(_) if max_depth > 0 => Err(A2aTextError::MessageNotObject),
            Shape::Text(_) | Shape::List(_) => Ok(String::new()),
            Shape::Object(_) => Err(A2aTextError::ArtifactsNotIndexed),
            Shape::Null | Shape::Flag(_) | Shape::Number(_) => Err(A2aTextError::ArtifactsNotSized),
        },
        _ => Ok(String::new()),
    }
}

mod tests;
