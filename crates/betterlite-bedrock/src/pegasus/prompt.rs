use serde::Deserialize;

use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PromptMessage {
    pub role: Option<String>,
    pub content: Option<Field>,
}

pub fn convert_messages_to_prompt(messages: &[PromptMessage]) -> Result<String, PegasusError> {
    let mut parts = Vec::with_capacity(messages.len());
    for message in messages {
        let role = message.role.as_deref().unwrap_or("user");
        let content = content_text(message.content.as_ref())?;
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
    match block.get("type").and_then(Field::as_str) {
        Some("text") => match block.get("text") {
            None | Some(Field::Null) => Ok(Some(String::new())),
            Some(Field::Text(text)) => Ok(Some(text.clone())),
            Some(_) => Err(PegasusError::NotText { field: "text" }),
        },
        Some("image_url") => Ok(Some("<image>".to_owned())),
        Some("video_url") => Ok(Some("<video>".to_owned())),
        Some("audio_url") => Ok(Some("<audio>".to_owned())),
        Some(_) | None => Ok(None),
    }
}
