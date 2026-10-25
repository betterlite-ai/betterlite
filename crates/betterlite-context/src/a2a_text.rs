use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type A2aObject = BTreeMap<String, A2aValue>;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum A2aValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(A2aObject),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum A2aTextError {
    #[error("A2A message parts must be an array")]
    PartsNotArray,
    #[error("A2A message part must be an object")]
    PartNotObject,
    #[error("A2A message must be an object")]
    MessageNotObject,
    #[error("A2A text part text must be a string")]
    TextNotString,
}

pub fn extract_text_from_a2a_message(
    message: Option<&A2aValue>,
    depth: usize,
    max_depth: usize,
) -> Result<String, A2aTextError> {
    if message.is_none() || depth >= max_depth {
        return Ok(String::new());
    }
    let message_value = message.ok_or(A2aTextError::MessageNotObject)?;
    let object = message_value
        .as_object()
        .ok_or(A2aTextError::MessageNotObject)?;
    extract_object_text(object, depth, max_depth)
}

fn extract_object_text(
    object: &A2aObject,
    depth: usize,
    max_depth: usize,
) -> Result<String, A2aTextError> {
    let empty_parts = A2aValue::Array(Vec::new());
    let parts_value = object.get("parts").unwrap_or(&empty_parts);
    let parts = parts_value.as_array().ok_or(A2aTextError::PartsNotArray)?;
    let mut text_parts = Vec::new();
    for part_value in parts {
        let part = part_value.as_object().ok_or(A2aTextError::PartNotObject)?;
        match part.get("kind") {
            Some(A2aValue::String(kind)) if kind == "text" => {
                let empty_text = A2aValue::String(String::new());
                let text = part.get("text").unwrap_or(&empty_text);
                text_parts.push(
                    text.as_string()
                        .ok_or(A2aTextError::TextNotString)?
                        .to_owned(),
                );
            }
            _ => {
                let nested_result = part
                    .get("parts")
                    .map(|_| {
                        let nested_object = A2aValue::Object(part.clone());
                        let next_depth =
                            depth.checked_add(1).ok_or(A2aTextError::MessageNotObject)?;
                        extract_text_from_a2a_message(Some(&nested_object), next_depth, max_depth)
                    })
                    .transpose()?;
                if let Some(nested_text) = nested_result.filter(|text| !text.is_empty()) {
                    text_parts.push(nested_text);
                }
            }
        }
    }
    Ok(text_parts.join(" "))
}

pub fn extract_text_from_a2a_response(
    response: &A2aObject,
    max_depth: usize,
) -> Result<String, A2aTextError> {
    let default_result = A2aValue::Object(BTreeMap::new());
    let result_value = response.get("result").unwrap_or(&default_result);
    let Some(result) = result_value.as_object() else {
        return Ok(String::new());
    };
    if result.contains_key("parts") {
        return extract_text_from_a2a_message(Some(result_value), 0, max_depth);
    }
    if let Some(message) = result.get("message").filter(|value| value.is_truthy()) {
        return extract_text_from_a2a_message(Some(message), 0, max_depth);
    }
    if let Some(artifact) = result.get("artifact").filter(|value| value.is_truthy())
        && artifact.as_object().is_some()
    {
        return extract_text_from_a2a_message(Some(artifact), 0, max_depth);
    }
    if let Some(status) = result.get("status").and_then(A2aValue::as_object)
        && let Some(message) = status.get("message").filter(|value| value.is_truthy())
    {
        return extract_text_from_a2a_message(Some(message), 0, max_depth);
    }
    if let Some(artifacts) = result.get("artifacts").filter(|value| value.is_truthy())
        && let Some(first) = artifacts.as_array().and_then(|items| items.first())
    {
        return extract_text_from_a2a_message(Some(first), 0, max_depth);
    }
    Ok(String::new())
}

impl A2aValue {
    const fn as_object(&self) -> Option<&A2aObject> {
        match self {
            Self::Object(object) => Some(object),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Array(_) => None,
        }
    }

    const fn as_array(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Array(items) => Some(items),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Object(_) => {
                None
            }
        }
    }

    fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Array(_) | Self::Object(_) => None,
        }
    }

    fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => *value,
            Self::Number(_) => true,
            Self::String(value) => !value.is_empty(),
            Self::Array(value) => !value.is_empty(),
            Self::Object(value) => !value.is_empty(),
        }
    }
}

mod tests;
