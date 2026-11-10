use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::error::BytezConfigError;

// LiteLLM reads only role and content from a caller's message and drops every other key
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedMessage {
    pub role: Option<String>,
    pub content: Vec<NormalizedBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum NormalizedBlock {
    Text { text: String },
    Image { url: String },
    Audio { url: String },
    Video { url: String },
}

pub fn normalize_messages(
    messages: &[RawMessage],
) -> Result<Vec<NormalizedMessage>, BytezConfigError> {
    let mut normalized: Vec<NormalizedMessage> = Vec::with_capacity(messages.len());
    for message in messages {
        normalized.push(normalize_message(message)?);
    }
    Ok(normalized)
}

fn normalize_message(message: &RawMessage) -> Result<NormalizedMessage, BytezConfigError> {
    let Some(content) = message.content.as_ref() else {
        return Err(BytezConfigError::ContentShape);
    };
    let mut blocks: Vec<NormalizedBlock> = Vec::new();
    push_blocks(content, &mut blocks)?;
    Ok(NormalizedMessage {
        role: message.role.clone(),
        content: blocks,
    })
}

fn push_blocks(content: &Json, blocks: &mut Vec<NormalizedBlock>) -> Result<(), BytezConfigError> {
    match content {
        Json::Text(text) => push_text(text.clone(), blocks),
        Json::Object(fields) => {
            blocks.push(block_of(fields)?);
            Ok(())
        }
        Json::List(items) => {
            for item in items {
                match item {
                    Json::Text(text) => push_text(text.clone(), blocks)?,
                    Json::Object(fields) => blocks.push(block_of(fields)?),
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                        return Err(BytezConfigError::ContentItemShape);
                    }
                }
            }
            Ok(())
        }
        Json::Null | Json::Bool(_) | Json::Number(_) => Err(BytezConfigError::ContentShape),
    }
}

// A bare string becomes a text block and then passes the same payload check as a caller's text block
fn push_text(text: String, blocks: &mut Vec<NormalizedBlock>) -> Result<(), BytezConfigError> {
    if text.is_empty() {
        return Err(BytezConfigError::ContentValue {
            name: String::from("text"),
        });
    }
    blocks.push(NormalizedBlock::Text { text });
    Ok(())
}

fn block_of(fields: &Object) -> Result<NormalizedBlock, BytezConfigError> {
    let Some(kind_value) = fields.get("type").filter(|value| value.is_truthy()) else {
        return Err(BytezConfigError::MissingContentType);
    };
    let kind = kind_value
        .as_str()
        .ok_or(BytezConfigError::UnknownContentType)?;
    match kind {
        "text" => Ok(NormalizedBlock::Text {
            text: payload(fields, "text")?,
        }),
        "image_url" => Ok(NormalizedBlock::Image {
            url: payload(fields, "url")?,
        }),
        "input_audio" => Ok(NormalizedBlock::Audio {
            url: payload(fields, "url")?,
        }),
        "video_url" => Ok(NormalizedBlock::Video {
            url: payload(fields, "url")?,
        }),
        "document" | "file" => Err(BytezConfigError::UnsupportedContentType {
            kind: kind.to_owned(),
        }),
        _ => Err(BytezConfigError::UnknownContentType),
    }
}

fn payload(fields: &Object, name: &str) -> Result<String, BytezConfigError> {
    fields
        .get(name)
        .and_then(Json::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| BytezConfigError::ContentValue {
            name: name.to_owned(),
        })
}

pub fn get_tokens_from_messages(messages: &[NormalizedMessage]) -> Result<i64, BytezConfigError> {
    let mut total: i64 = 0;
    for message in messages {
        for block in &message.content {
            let count = match block {
                NormalizedBlock::Text { text } => i64::try_from(text.split(' ').count())
                    .map_err(|_conversion| BytezConfigError::UsageOverflow)?,
                NormalizedBlock::Image { .. }
                | NormalizedBlock::Audio { .. }
                | NormalizedBlock::Video { .. } => 1,
            };
            total = total
                .checked_add(count)
                .ok_or(BytezConfigError::UsageOverflow)?;
        }
    }
    Ok(total)
}
