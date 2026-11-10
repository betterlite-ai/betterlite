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

// open_ai_to_bytez_content_item_map: the caller's type, the Bytez type and the payload key
const CONTENT_MAP: [(&str, &str, &str); 4] = [
    ("text", "text", "text"),
    ("image_url", "image", "url"),
    ("input_audio", "audio", "url"),
    ("video_url", "video", "url"),
];

// the two OpenAI block types Bytez knows and refuses
const UNSUPPORTED_CONTENT: [&str; 2] = ["document", "file"];

pub fn adapt_messages_to_bytez_standard(
    messages: &[RawMessage],
) -> Result<Vec<NormalizedMessage>, BytezConfigError> {
    messages.iter().map(normalize_message).collect()
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
    let Some((_, alias, value_name)) = CONTENT_MAP.iter().find(|(name, ..)| *name == kind) else {
        if UNSUPPORTED_CONTENT.contains(&kind) {
            return Err(BytezConfigError::UnsupportedContentType {
                kind: kind.to_owned(),
            });
        }
        return Err(BytezConfigError::UnknownContentType);
    };
    let value = fields
        .get(*value_name)
        .and_then(Json::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BytezConfigError::ContentValue {
            name: (*value_name).to_owned(),
        })?
        .to_owned();
    match *alias {
        "text" => Ok(NormalizedBlock::Text { text: value }),
        "image" => Ok(NormalizedBlock::Image { url: value }),
        "audio" => Ok(NormalizedBlock::Audio { url: value }),
        _ => Ok(NormalizedBlock::Video { url: value }),
    }
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
