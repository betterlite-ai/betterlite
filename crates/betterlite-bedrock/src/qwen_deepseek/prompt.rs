use betterlite_logging::analytics::python::{FormatError, repr};
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

pub(crate) const ASSISTANT_START: &str = "<|im_start|>assistant\n";
pub(crate) const IM_END: &str = "<|im_end|>";

const VISION_PLACEHOLDER: &str = "<|vision_start|><|image_pad|><|vision_end|>";

// Each variant stands for the Python exception the source template raises on that value.
#[derive(Debug, Error)]
pub enum PromptError {
    #[error("message {index} is not a JSON object")]
    MessageNotObject { index: usize },
    #[error("content item {item} of message {index} is not a JSON object")]
    ContentItemNotObject { index: usize, item: usize },
    #[error("the text of content item {item} in message {index} is not a string")]
    ContentItemText { index: usize, item: usize },
    #[error("tool call {call} of message {index} is not a JSON object")]
    ToolCallNotObject { index: usize, call: usize },
    #[error("the function of tool call {call} in message {index} is not a JSON object")]
    FunctionNotObject { index: usize, call: usize },
    #[error("cannot format a prompt value")]
    Format(#[source] FormatError),
    #[error("cannot encode the request body")]
    Encode(#[source] serde_json::Error),
}

// AmazonQwen3Config._convert_messages_to_prompt: the fixed local template, with no template download.
pub fn convert_messages_to_prompt(messages: &[Json]) -> Result<String, PromptError> {
    let mut parts = Vec::with_capacity(messages.len().saturating_add(1));
    for (index, message) in messages.iter().enumerate() {
        if let Some(block) = message_block(message, index)? {
            parts.push(block);
        }
    }
    parts.push(ASSISTANT_START.to_owned());
    Ok(parts.join("\n"))
}

fn message_block(message: &Json, index: usize) -> Result<Option<String>, PromptError> {
    let Json::Object(fields) = message else {
        return Err(PromptError::MessageNotObject { index });
    };
    let role = fields.get("role").and_then(Json::as_str).unwrap_or("");
    if role == "assistant" {
        return assistant_block(fields, index).map(Some);
    }
    if !matches!(role, "system" | "user" | "tool") {
        return Ok(None);
    }
    let content = match fields.get("content") {
        Some(Json::List(items)) if role == "user" => user_items(items, index)?,
        value => content_text(value)?,
    };
    Ok(Some(format!("<|im_start|>{role}\n{content}{IM_END}")))
}

// A truthy list of tool calls replaces the assistant content with one block per call.
fn assistant_block(fields: &Object, index: usize) -> Result<String, PromptError> {
    if let Some(Json::List(calls)) = fields.get("tool_calls")
        && !calls.is_empty()
    {
        let mut blocks = Vec::with_capacity(calls.len());
        for (call, tool_call) in calls.iter().enumerate() {
            blocks.push(tool_call_block(tool_call, index, call)?);
        }
        return Ok(blocks.join("\n"));
    }
    let content = content_text(fields.get("content"))?;
    Ok(format!("<|im_start|>assistant\n{content}{IM_END}"))
}

fn tool_call_block(tool_call: &Json, index: usize, call: usize) -> Result<String, PromptError> {
    let Json::Object(fields) = tool_call else {
        return Err(PromptError::ToolCallNotObject { index, call });
    };
    let function_fields = match fields.get("function") {
        Some(Json::Object(object)) => Some(object),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(PromptError::FunctionNotObject { index, call });
        }
        None => None,
    };
    let name = content_text(function_fields.and_then(|fields| fields.get("name")))?;
    let arguments = content_text(function_fields.and_then(|fields| fields.get("arguments")))?;
    Ok(format!(
        "{ASSISTANT_START}<tool_call>\n{{\"name\": \"{name}\", \"arguments\": \"{arguments}\"}}\n</tool_call>{IM_END}"
    ))
}

// A missing content is empty; an explicit null renders Python's None.
fn content_text(content: Option<&Json>) -> Result<String, PromptError> {
    let Some(value) = content else {
        return Ok(String::new());
    };
    display_text(value)
}

// Text items are concatenated; every image_url item becomes the vision placeholder and other kinds vanish.
fn user_items(items: &[Json], index: usize) -> Result<String, PromptError> {
    let mut texts = Vec::with_capacity(items.len());
    let mut non_text = None;
    for (item, value) in items.iter().enumerate() {
        let Json::Object(fields) = value else {
            return Err(PromptError::ContentItemNotObject { index, item });
        };
        let kind = fields.get("type").and_then(Json::as_str);
        if kind == Some("text") {
            match fields.get("text") {
                Some(Json::Text(text)) => texts.push(text.clone()),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
                ) => {
                    non_text = non_text.or(Some(item));
                    texts.push(String::new());
                }
                None => texts.push(String::new()),
            }
        } else if kind == Some("image_url") {
            texts.push(VISION_PLACEHOLDER.to_owned());
        }
    }
    if let Some(item) = non_text {
        return Err(PromptError::ContentItemText { index, item });
    }
    Ok(texts.concat())
}

// An f-string renders a string as sent and any other value as its Python display form.
fn display_text(value: &Json) -> Result<String, PromptError> {
    if let Json::Text(text) = value {
        return Ok(text.clone());
    }
    repr(value).map_err(PromptError::Format)
}
