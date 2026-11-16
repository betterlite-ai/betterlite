use std::collections::BTreeMap;

use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::reasoning_fields::content_split::extract_reasoning_content;
use serde_json::Value;

use crate::databricks_chat::content::{extract_citations, extract_content_str, extract_reasoning};
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::body::{
    choices_of, missing_key, native_fields, parse_body, required_integer, required_text,
    response_model, usage_of, wrong_type,
};
use crate::ollama_chat::ids::IdSource;

mod body;
pub mod reply;
mod tests;
mod tool_calls;
pub(crate) mod usage;

use reply::{Choice, Reply, ReplyMessage, ReplyToolCall};

pub struct ResponseInput<'value> {
    pub body: &'value str,
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub custom_llm_provider: Option<&'value str>,
    pub json_mode: Option<bool>,
    pub ids: &'value dyn IdSource,
}

// transform_response: parse the body, then each choice through _transform_dbrx_choices
pub fn transform_response(input: ResponseInput<'_>) -> Result<Reply, DatabricksChatError> {
    let body = parse_body(&input)?;
    let id = required_text(&body, "id")?;
    let created = required_integer(&body, "created")?;
    let model = response_model(&input, &body)?;
    let usage = usage_of(&body)?;
    let choices = choices_of(&body)?;
    let mut transformed = Vec::with_capacity(choices.len());
    for choice in choices {
        let json_mode = input.json_mode.unwrap_or(false);
        transformed.push(transform_choice(choice, json_mode, input.ids)?);
    }
    Ok(Reply {
        id: id.to_owned(),
        created,
        model,
        object: String::from("chat.completion"),
        system_fingerprint: None,
        choices: transformed,
        usage,
    })
}

// One DatabricksChoice: the tool call repair, the json mode shortcut, then the message transform
fn transform_choice(
    value: &Json,
    json_mode: bool,
    ids: &dyn IdSource,
) -> Result<Choice, DatabricksChatError> {
    let Some(fields) = value.as_object() else {
        return Err(wrong_type("choice", "an object"));
    };
    let message = message_of(fields)?;
    let index = index_of(fields)?;
    let calls = tool_calls::normalize(message.get("tool_calls"), ids)?;
    let repaired = match &calls {
        Some(calls) => tool_calls::repair(calls)?,
        None => Vec::new(),
    };
    if let Some(content) = tool_calls::json_mode_content(&repaired, json_mode) {
        return Ok(Choice {
            finish_reason: String::from("stop"),
            index,
            message: ReplyMessage {
                content: Some(content),
                role: String::from("assistant"),
                tool_calls: None,
                function_call: None,
                reasoning_content: None,
                thinking_blocks: None,
                provider_specific_fields: None,
            },
            provider_specific_fields: None,
        });
    }
    let message = translated_message(message, calls)?;
    let (finish_reason, native) = finish_reason_of(fields)?;
    Ok(Choice {
        finish_reason,
        index,
        message,
        provider_specific_fields: native_fields(native)?,
    })
}

fn message_of(fields: &Object) -> Result<&Object, DatabricksChatError> {
    match fields.get("message") {
        Some(Json::Object(message)) => Ok(message),
        None => Err(missing_key("message")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(wrong_type("message of a choice", "an object"))
        }
    }
}

fn index_of(fields: &Object) -> Result<i64, DatabricksChatError> {
    match fields.get("index") {
        None => Err(missing_key("index")),
        Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => match number.as_i64() {
            Some(index) => Ok(index),
            None => Err(wrong_type("index of a choice", "an integer")),
        },
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(wrong_type("index of a choice", "an integer"))
        }
    }
}

// Choices maps a string finish_reason and defaults a null one; the key is only read without json mode
fn finish_reason_of(
    fields: &Object,
) -> Result<(String, BTreeMap<String, Value>), DatabricksChatError> {
    let raw = match fields.get("finish_reason") {
        None => return Err(missing_key("finish_reason")),
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(wrong_type("finish_reason of a choice", "a string"));
        }
    };
    Ok(choice_finish_reason(raw))
}

// Message(role="assistant", ...) from the message content, its reasoning blocks and its citations
fn translated_message(
    message: &Object,
    calls: Option<Vec<ReplyToolCall>>,
) -> Result<ReplyMessage, DatabricksChatError> {
    let content = message
        .get("content")
        .ok_or_else(|| missing_key("content"))?;
    let (block_reasoning, blocks) = extract_reasoning(Some(content))?;
    let content_str = extract_content_str(Some(content))?;
    let (reasoning_content, content) = match block_reasoning {
        Some(reasoning) => (Some(reasoning), content_str),
        None => split_reasoning(message, content_str)?,
    };
    let citations = extract_citations(Some(content))?;
    let provider_specific_fields = match citations {
        Some(citations) => {
            let mut fields = Object::new();
            fields.insert("citations".to_owned(), Json::List(citations));
            Some(fields)
        }
        None => None,
    };
    Ok(ReplyMessage {
        content,
        role: String::from("assistant"),
        tool_calls: calls,
        function_call: None,
        reasoning_content,
        thinking_blocks: blocks,
        provider_specific_fields,
    })
}

// resolve_reasoning_and_content falls back to _extract_reasoning_content over the extracted text
fn split_reasoning(
    message: &Object,
    content: Option<String>,
) -> Result<(Option<String>, Option<String>), DatabricksChatError> {
    let mut fields = Object::new();
    for name in ["reasoning_content", "reasoning"] {
        let Some(value) = message.get(name) else {
            continue;
        };
        match value {
            Json::Text(_) | Json::Null => {
                fields.insert(name.to_owned(), value.clone());
            }
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(wrong_type(name, "a string"));
            }
        }
    }
    let synthetic = Message {
        role: String::from("assistant"),
        content: content.clone().map(Content::Text),
        name: None,
        provider_specific_fields: fields,
    };
    let (reasoning, resolved) = extract_reasoning_content(&synthetic);
    let resolved = match resolved {
        Some(Content::Text(text)) => Some(text),
        Some(Content::Parts(_)) | None => None,
    };
    Ok((reasoning, resolved))
}
