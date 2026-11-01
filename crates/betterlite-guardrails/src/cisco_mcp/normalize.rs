use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

pub struct NormalizedResponse {
    pub id_override: Option<Box<RawValue>>,
    pub result: Option<ResultValue>,
    pub error: Option<Box<RawValue>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ResultValue {
    Raw(Box<RawValue>),
    Built(McpResult),
}

#[derive(Debug, Clone, Serialize)]
pub struct McpResult {
    pub content: Vec<ContentItem>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Box<RawValue>>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ContentItem {
    Passthrough(Box<RawValue>),
    Fallback {
        #[serde(rename = "type")]
        kind: &'static str,
        text: String,
    },
}

#[must_use]
pub fn normalize_response(response: &Json) -> Option<NormalizedResponse> {
    match response {
        Json::Object(fields) => normalize_object(fields),
        Json::List(items) => normalize_list(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
    }
}

fn normalize_object(fields: &Fields) -> Option<NormalizedResponse> {
    if matches!(Json::field(fields, "jsonrpc"), Json::Text(version) if version == "2.0") {
        return Some(NormalizedResponse {
            id_override: id_for_override(fields.get("id").map(AsRef::as_ref)),
            result: fields.get("result").cloned().map(ResultValue::Raw),
            error: fields.get("error").cloned(),
        });
    }
    if let Some(raw) = fields.get("result")
        && matches!(Json::parse(raw.get()), Json::Object(_))
    {
        return Some(NormalizedResponse {
            id_override: id_for_override_with_fallback(fields.get("id").map(AsRef::as_ref)),
            result: Some(ResultValue::Raw(raw.clone())),
            error: None,
        });
    }
    if let Json::List(content) = Json::field(fields, "content") {
        return Some(NormalizedResponse {
            id_override: id_for_override_with_fallback(fields.get("id").map(AsRef::as_ref)),
            result: Some(ResultValue::Built(build_result(&content, Some(fields)))),
            error: None,
        });
    }
    None
}

fn normalize_list(items: &[Box<RawValue>]) -> Option<NormalizedResponse> {
    if is_pair_list(items) {
        let fields = pairs_to_fields(items);
        let Json::List(content) = Json::field(&fields, "content") else {
            return None;
        };
        return Some(NormalizedResponse {
            id_override: None,
            result: Some(ResultValue::Built(build_result(&content, Some(&fields)))),
            error: None,
        });
    }
    Some(NormalizedResponse {
        id_override: None,
        result: Some(ResultValue::Built(build_result(items, None))),
        error: None,
    })
}

#[must_use]
pub fn build_result(content: &[Box<RawValue>], source: Option<&Fields>) -> McpResult {
    let items = content
        .iter()
        .map(|item| serialize_content_item(item))
        .collect();
    let structured_content = source
        .and_then(|fields| source_field_raw(fields, "structuredContent", "structured_content"));
    let is_error = source.and_then(|fields| source_field_bool(fields, "isError", "is_error"));
    McpResult {
        content: items,
        structured_content,
        is_error,
    }
}

#[must_use]
pub fn serialize_content_item(item: &RawValue) -> ContentItem {
    match Json::parse(item.get()) {
        Json::Object(_) => ContentItem::Passthrough(item.to_owned()),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            ContentItem::Fallback {
                kind: "text",
                text: other.python_str(),
            }
        }
    }
}

fn source_field_raw(fields: &Fields, camel: &str, snake: &str) -> Option<Box<RawValue>> {
    let raw = fields.get(camel).or_else(|| fields.get(snake))?;
    (!matches!(Json::parse(raw.get()), Json::Null)).then(|| raw.clone())
}

fn source_field_bool(fields: &Fields, camel: &str, snake: &str) -> Option<bool> {
    let raw = fields.get(camel).or_else(|| fields.get(snake))?;
    match Json::parse(raw.get()) {
        Json::Bool(flag) => Some(flag),
        Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn id_for_override(id: Option<&RawValue>) -> Option<Box<RawValue>> {
    let present_id = id?;
    if matches!(Json::parse(present_id.get()), Json::Null) {
        return None;
    }
    if matches!(Json::parse(present_id.get()), Json::Text(value) if value == "litellm-mcp") {
        return None;
    }
    Some(present_id.to_owned())
}

fn id_for_override_with_fallback(id: Option<&RawValue>) -> Option<Box<RawValue>> {
    let present_id = id?;
    if !Json::parse(present_id.get()).truthy() {
        return None;
    }
    id_for_override(Some(present_id))
}

pub(crate) fn is_pair_list(items: &[Box<RawValue>]) -> bool {
    if items.is_empty() {
        return false;
    }
    for item in items {
        if pair_key(item.get()).is_none() {
            return false;
        }
    }
    true
}

pub(crate) fn pairs_to_fields(items: &[Box<RawValue>]) -> Fields {
    let mut fields = Fields::new();
    for item in items {
        let Json::List(pair) = Json::parse(item.get()) else {
            continue;
        };
        let (Some(key_raw), Some(value_raw)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let Json::Text(key) = Json::parse(key_raw.get()) else {
            continue;
        };
        if pair.len() == 2 {
            fields.insert(key, value_raw.clone());
        }
    }
    fields
}

fn pair_key(text: &str) -> Option<String> {
    let Json::List(pair) = Json::parse(text) else {
        return None;
    };
    if pair.len() != 2 {
        return None;
    }
    match Json::parse(pair.first()?.get()) {
        Json::Text(key) => Some(key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

mod tests;
