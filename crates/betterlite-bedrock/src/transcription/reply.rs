use betterlite_models::decisions::Json;
use serde::{Deserialize, Deserializer, Serialize};

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
    pub content: Vec<Block>,
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
pub enum Block {
    Text(String),
    Other,
}

impl<'de> Deserialize<'de> for Block {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Json::deserialize(deserializer)? {
            Json::Object(fields) => match fields.get("text") {
                Some(Json::Text(text)) => Ok(Self::Text(text.clone())),
                Some(_) => Err(serde::de::Error::custom(
                    "invalid type for `text`, expected a string",
                )),
                None => Ok(Self::Other),
            },
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Ok(Self::Other)
            }
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
    let mut text = String::new();
    for block in reply.output.message.content {
        match block {
            Block::Text(part) => text.push_str(&part),
            Block::Other => return Err(TranscriptionError::NonTextContent),
        }
    }
    Ok(TranscriptionText { text })
}
