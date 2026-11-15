use serde::Deserialize;

use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct PromptMessage(Params);

pub fn convert_messages_to_prompt(messages: &[PromptMessage]) -> Result<String, PegasusError> {
    let mut parts = Vec::with_capacity(messages.len());
    for message in messages {
        let role = match message.0.get("role") {
            None => "user",
            Some(Field::Text(text)) => text.as_str(),
            Some(Field::Null) => "None",
            Some(_) => return Err(PegasusError::NotText { field: "role" }),
        };
        let content = content_text(message.0.get("content"))?;
        parts.push(format!("{role}: {content}"));
    }
    Ok(parts.join("\n").trim().to_owned())
}

fn content_text(content: Option<&Field>) -> Result<String, PegasusError> {
    match content {
        None => Ok(String::new()),
        Some(Field::Null) => Ok("None".to_owned()),
        Some(Field::Text(text)) => Ok(text.clone()),
        Some(Field::List(items)) => join_fragments(items),
        Some(Field::Bool(_) | Field::Int(_) | Field::Float(_) | Field::Mapping(_)) => {
            Err(PegasusError::NotText { field: "content" })
        }
    }
}

fn join_fragments(items: &[Field]) -> Result<String, PegasusError> {
    let mut fragments = Vec::with_capacity(items.len());
    for item in items {
        if let Some(fragment) = part_text(item)? {
            fragments.push(fragment);
        }
    }
    Ok(fragments.join(" "))
}

fn part_text(item: &Field) -> Result<Option<String>, PegasusError> {
    match item {
        Field::Text(text) => Ok(Some(text.clone())),
        Field::Mapping(block) => block_text(block),
        Field::Null | Field::Bool(_) | Field::Int(_) | Field::Float(_) | Field::List(_) => Ok(None),
    }
}

fn block_text(block: &Params) -> Result<Option<String>, PegasusError> {
    let kind = block.get("type").and_then(Field::as_str);
    if kind == Some("text") {
        return match block.get("text") {
            None | Some(Field::Null) => Ok(Some(String::new())),
            Some(Field::Text(text)) => Ok(Some(text.clone())),
            Some(_) => Err(PegasusError::NotText { field: "text" }),
        };
    }
    let placeholder = match kind {
        Some("image_url") => "<image>",
        Some("video_url") => "<video>",
        Some("audio_url") => "<audio>",
        _ => return Ok(None),
    };
    Ok(Some(placeholder.to_owned()))
}
