use std::collections::BTreeMap;

use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::reasoning_fields::content_split::extract_reasoning_content;

use crate::databricks_chat::content::{extract_citations, extract_content_str, extract_reasoning};
use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::keyed::Keyed;
use crate::databricks_chat::response::usage::usage_from;
use crate::databricks_chat::response::wire::{DatabricksChoice, DatabricksMessage, parse_body};
use crate::ollama_chat::ids::IdSource;

pub mod reply;
mod tests;
mod tool_calls;
pub(crate) mod usage;
mod wire;

use reply::{Choice, Reply, ReplyMessage, ReplyToolCall};

pub struct ResponseInput<'value> {
    pub body: &'value str,
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub custom_llm_provider: Option<&'value str>,
    pub json_mode: bool,
    pub ids: &'value dyn IdSource,
}

// transform_response: parse the body, then each choice through _transform_dbrx_choices
pub fn transform_response(input: ResponseInput<'_>) -> Result<Reply, DatabricksChatError> {
    let body = parse_body(&input)?;
    let provider = match input.custom_llm_provider {
        Some(name) if !name.is_empty() => name,
        None | Some(_) => "databricks",
    };
    let mut choices = Vec::with_capacity(body.choices.len());
    for choice in body.choices {
        choices.push(transform_choice(choice, input.json_mode, input.ids)?);
    }
    Ok(Reply {
        id: body.id,
        created: body.created,
        model: format!("{provider}/{}", body.model.unwrap_or_default()),
        object: String::from("chat.completion"),
        system_fingerprint: None,
        choices,
        usage: usage_from(&body.usage)?,
    })
}

// One DatabricksChoice: the tool call repair, the json mode shortcut, then the message transform
fn transform_choice(
    choice: DatabricksChoice,
    json_mode: bool,
    ids: &dyn IdSource,
) -> Result<Choice, DatabricksChatError> {
    let index = match choice.index {
        Keyed::Given(given) => given,
        Keyed::Null => 0,
        Keyed::Missing => return Err(DatabricksChatError::MissingKey { key: "index" }),
    };
    let calls = tool_calls::normalize(choice.message.tool_calls.as_deref(), ids)?;
    let repaired = match &calls {
        Some(normalized) => tool_calls::repair(normalized)?,
        None => Vec::new(),
    };
    if let Some(content) = tool_calls::json_mode_content(&repaired, json_mode) {
        let json_message = ReplyMessage {
            content: Some(content),
            role: String::from("assistant"),
            tool_calls: None,
            function_call: None,
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
        };
        return Ok(Choice {
            finish_reason: String::from("stop"),
            index,
            message: json_message,
            provider_specific_fields: None,
        });
    }
    let message = translated_message(&choice.message, calls)?;
    let (finish_reason, provider_specific_fields) = finish_reason_of(choice.finish_reason)?;
    Ok(Choice {
        finish_reason,
        index,
        message,
        provider_specific_fields,
    })
}

// Choices maps a given finish_reason and keeps the raw one when the mapping changed it
fn finish_reason_of(raw: Keyed<String>) -> Result<(String, Option<Object>), DatabricksChatError> {
    let given = match raw {
        Keyed::Given(text) => Some(text),
        Keyed::Null => None,
        Keyed::Missing => {
            return Err(DatabricksChatError::MissingKey {
                key: "finish_reason",
            });
        }
    };
    let (finish_reason, native) = choice_finish_reason(given.as_deref());
    let mut native_fields = Object::new();
    for (name, value) in native {
        native_fields.insert(
            name,
            serde_json::from_value(value).map_err(DatabricksChatError::Decode)?,
        );
    }
    Ok((
        finish_reason,
        (!native_fields.is_empty()).then_some(native_fields),
    ))
}

// Message(role="assistant", ...) from the message content, its reasoning blocks and its citations
fn translated_message(
    message: &DatabricksMessage,
    calls: Option<Vec<ReplyToolCall>>,
) -> Result<ReplyMessage, DatabricksChatError> {
    let content = match &message.content {
        Keyed::Given(content) => Some(content),
        Keyed::Null => None,
        Keyed::Missing => return Err(DatabricksChatError::MissingKey { key: "content" }),
    };
    let (block_reasoning, thinking_blocks) = extract_reasoning(content);
    let content_str = extract_content_str(content);
    let (reasoning_content, text) = match block_reasoning {
        Some(reasoning) => (Some(reasoning), content_str),
        None => split_reasoning(message, content_str),
    };
    let provider_specific_fields = extract_citations(content)?.map(|citations| {
        let mut fields = Object::new();
        fields.insert(String::from("citations"), Json::List(citations));
        fields
    });
    Ok(ReplyMessage {
        content: text,
        role: String::from("assistant"),
        tool_calls: calls,
        function_call: None,
        reasoning_content,
        thinking_blocks,
        provider_specific_fields,
    })
}

// resolve_reasoning_and_content falls back to _extract_reasoning_content over the extracted text
fn split_reasoning(
    message: &DatabricksMessage,
    content: Option<String>,
) -> (Option<String>, Option<String>) {
    let mut fields = Object::new();
    for (name, value) in [
        ("reasoning_content", &message.reasoning_content),
        ("reasoning", &message.reasoning),
    ] {
        match value {
            Keyed::Given(text) => {
                fields.insert(name.to_owned(), Json::Text(text.clone()));
            }
            Keyed::Null => {
                fields.insert(name.to_owned(), Json::Null);
            }
            Keyed::Missing => {}
        }
    }
    let synthetic = Message {
        role: String::from("assistant"),
        content: content.map(Content::Text),
        name: None,
        provider_specific_fields: fields,
    };
    match extract_reasoning_content(&synthetic) {
        (reasoning, Some(Content::Text(text))) => (reasoning, Some(text)),
        (reasoning, Some(Content::Parts(_)) | None) => (reasoning, None),
    }
}
