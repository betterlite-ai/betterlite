use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::error::TranscriptionError;

#[derive(Debug, Deserialize)]
pub struct ConverseReply {
    pub output: Output,
    #[serde(default)]
    pub usage: Usage,
    #[serde(rename = "stopReason")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Output {
    pub message: Message,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub content: Vec<Json>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    pub total_tokens: Option<u64>,
}

#[derive(Debug)]
enum ContentBlock {
    Text(String),
    Other,
}

// a text key of the wrong type fails the whole reply before a non-text block is reported
fn content_block(block: Json) -> Result<ContentBlock, TranscriptionError> {
    match block {
        Json::Object(fields) => match fields.get("text") {
            Some(Json::Text(text)) => Ok(ContentBlock::Text(text.clone())),
            Some(_) => Err(TranscriptionError::InvalidTextType),
            None => Ok(ContentBlock::Other),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Ok(ContentBlock::Other)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionText {
    pub text: String,
}

pub fn decode_transcription_reply(bytes: &[u8]) -> Result<TranscriptionText, TranscriptionError> {
    let reply: ConverseReply =
        serde_json::from_slice(bytes).map_err(TranscriptionError::InvalidResponse)?;
    let blocks = reply
        .output
        .message
        .content
        .into_iter()
        .map(content_block)
        .collect::<Result<Vec<_>, _>>()?;
    let mut text = String::new();
    for block in blocks {
        match block {
            ContentBlock::Text(part) => text.push_str(&part),
            ContentBlock::Other => return Err(TranscriptionError::NonTextContent),
        }
    }
    Ok(TranscriptionText { text })
}
