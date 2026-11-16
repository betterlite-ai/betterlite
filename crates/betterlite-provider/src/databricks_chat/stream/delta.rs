use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::databricks_chat::content::{ThinkingBlock, extract_content_str, extract_reasoning};
use crate::databricks_chat::error::DatabricksChatError;

mod tool_calls;

// Delta(**delta) consumes these kwargs; everything else lands on the delta as an extra
const NAMED: [&str; 11] = [
    "content",
    "role",
    "function_call",
    "tool_calls",
    "audio",
    "images",
    "annotations",
    "reasoning_items",
    "reasoning_content",
    "thinking_blocks",
    "provider_specific_fields",
];

// LiteLLM's Delta: the five OpenAI keys always dump, the rest only when they carry something
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Delta {
    #[serde(flatten)]
    pub extra: Object,
    pub content: Option<String>,
    pub role: Option<String>,
    pub function_call: Option<Json>,
    pub tool_calls: Json,
    pub audio: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_items: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<Json>,
}

// build_delta turns the delta the chunk parser mutated into the typed Delta it validates
pub(super) fn build_delta(delta: &Object) -> Result<Delta, DatabricksChatError> {
    let content = extract_content_str(delta.get("content"))?;
    let (block_reasoning, thinking_blocks) = extract_reasoning(delta.get("content"))?;
    let mut extra = stripped(delta);
    let reasoning_content = resolved_reasoning(delta, block_reasoning, &mut extra)?;
    Ok(Delta {
        extra,
        content,
        role: role_of(delta)?,
        function_call: function_call_of(delta.get("function_call"))?,
        tool_calls: tool_calls::normalize(delta.get("tool_calls"))?,
        audio: delta.get("audio").cloned().unwrap_or(Json::Null),
        images: images_of(delta.get("images"))?,
        annotations: non_null(delta.get("annotations")),
        reasoning_items: non_null(delta.get("reasoning_items")),
        reasoning_content,
        thinking_blocks,
        provider_specific_fields: provider_fields_of(delta)?,
    })
}

// The block reasoning wins; then the delta's own reasoning_content, then a mapped reasoning key
fn resolved_reasoning(
    delta: &Object,
    block: Option<String>,
    extra: &mut Object,
) -> Result<Option<String>, DatabricksChatError> {
    if let Some(text) = block {
        return Ok(Some(text));
    }
    match delta.get("reasoning_content") {
        None | Some(Json::Null) => mapped_reasoning(delta, extra),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "reasoning_content of a Databricks stream delta",
            "a string",
        )),
    }
}

// Delta maps a reasoning key onto reasoning_content when the chunk parser left it unset
fn mapped_reasoning(
    delta: &Object,
    extra: &mut Object,
) -> Result<Option<String>, DatabricksChatError> {
    let Some(value) = delta.get("reasoning") else {
        return Ok(None);
    };
    extra.shift_remove("reasoning");
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        _ => Err(DatabricksChatError::wrong_type(
            "reasoning of a Databricks stream delta",
            "a string",
        )),
    }
}

fn stripped(delta: &Object) -> Object {
    let mut extra = delta.clone();
    for name in NAMED {
        extra.shift_remove(name);
    }
    extra
}

fn role_of(delta: &Object) -> Result<Option<String>, DatabricksChatError> {
    match delta.get("role") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "role of a Databricks stream delta",
            "a string",
        )),
    }
}

// FunctionCall(**call): arguments is required, the name stays optional, extras survive
fn function_call_of(value: Option<&Json>) -> Result<Option<Json>, DatabricksChatError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let fields = match value {
        Json::Null => return Ok(None),
        Json::Object(fields) => fields,
        _ => {
            return Err(DatabricksChatError::wrong_type(
                "function_call of a Databricks stream delta",
                "an object",
            ));
        }
    };
    let mut call = Object::new();
    insert_arguments(&mut call, fields)?;
    insert_name(&mut call, fields)?;
    copy_extra(fields, &mut call, &["arguments", "name"]);
    Ok(Some(Json::Object(call)))
}

fn insert_arguments(call: &mut Object, fields: &Object) -> Result<(), DatabricksChatError> {
    match fields.get("arguments") {
        Some(Json::Text(text)) => {
            call.insert(String::from("arguments"), Json::Text(text.clone()));
            Ok(())
        }
        None => Err(DatabricksChatError::missing("function_call", "arguments")),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "arguments of a function_call",
            "a string",
        )),
    }
}

fn insert_name(call: &mut Object, fields: &Object) -> Result<(), DatabricksChatError> {
    match fields.get("name") {
        None => Ok(()),
        Some(Json::Null) => {
            call.insert(String::from("name"), Json::Null);
            Ok(())
        }
        Some(Json::Text(text)) => {
            call.insert(String::from("name"), Json::Text(text.clone()));
            Ok(())
        }
        Some(_) => Err(DatabricksChatError::wrong_type(
            "name of a function_call",
            "a string",
        )),
    }
}

fn images_of(value: Option<&Json>) -> Result<Option<Json>, DatabricksChatError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) if items.is_empty() => Ok(None),
        Some(list @ Json::List(_)) => Ok(Some(list.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "images of a Databricks stream delta",
            "a list",
        )),
    }
}

fn non_null(value: Option<&Json>) -> Option<Json> {
    match value {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

fn provider_fields_of(delta: &Object) -> Result<Option<Json>, DatabricksChatError> {
    match delta.get("provider_specific_fields") {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(Json::Null)),
        Some(fields @ Json::Object(_)) => Ok(Some(fields.clone())),
        Some(_) => Err(DatabricksChatError::wrong_type(
            "provider_specific_fields of a Databricks stream delta",
            "an object",
        )),
    }
}

pub(super) fn copy_extra(fields: &Object, target: &mut Object, named: &[&str]) {
    for (key, value) in fields {
        if named.contains(&key.as_str()) {
            continue;
        }
        target.insert(key.clone(), value.clone());
    }
}
