use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const DEFAULT_AUDIO_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AssemblyError {
    #[error("content field is not a string")]
    NonStringContent,
    #[error("invalid audio base64: {0}")]
    InvalidAudioBase64(String),
    #[error("decoded audio exceeds {limit} bytes")]
    AudioTooLarge { limit: usize },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Chunk {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Choice {
    #[serde(default)]
    pub delta: Option<Delta>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub reasoning_content: Option<Value>,
    #[serde(default)]
    pub thinking_blocks: Option<Value>,
    #[serde(default)]
    pub audio: Option<AudioDelta>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct AudioDelta {
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub transcript: Option<Value>,
    #[serde(default)]
    pub expires_at: Option<Value>,
    #[serde(default)]
    pub id: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum ThinkingBlock {
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    #[serde(rename = "redacted_thinking")]
    Redacted { data: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AudioResponse {
    pub data: String,
    pub expires_at: Value,
    pub transcript: String,
    pub id: Option<String>,
}

pub fn get_combined_content(
    chunks: &[Chunk],
    field: ContentField,
) -> Result<String, AssemblyError> {
    let mut combined = String::new();
    for chunk in chunks {
        for choice in &chunk.choices {
            let Some(delta) = &choice.delta else { continue };
            let value = match field {
                ContentField::Content => delta.content.as_ref(),
                ContentField::Reasoning => delta.reasoning_content.as_ref(),
            };
            let Some(value) = value else { continue };
            let Some(text) = value.as_str() else {
                if value.is_null() {
                    continue;
                }
                return Err(AssemblyError::NonStringContent);
            };
            combined.push_str(text);
        }
    }
    Ok(combined)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentField {
    Content,
    Reasoning,
}

pub fn get_combined_thinking_content(chunks: &[Chunk]) -> Option<Vec<ThinkingBlock>> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut signature: Option<String> = None;
    for chunk in chunks {
        for choice in &chunk.choices {
            let Some(Some(Value::Array(thinking_blocks))) = choice
                .delta
                .as_ref()
                .map(|delta| delta.thinking_blocks.as_ref())
            else {
                continue;
            };
            for block in thinking_blocks {
                let Value::Object(fields) = block else {
                    continue;
                };
                if fields.get("type").and_then(Value::as_str) == Some("redacted_thinking") {
                    flush_thinking(&mut blocks, &mut text, &mut signature);
                    if let Some(data) = fields
                        .get("data")
                        .and_then(Value::as_str)
                        .filter(|data| !data.is_empty())
                    {
                        blocks.push(ThinkingBlock::Redacted {
                            data: data.to_owned(),
                        });
                    }
                    continue;
                }
                if let Some(value) = fields
                    .get("thinking")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    text.push_str(value);
                }
                if let Some(value) = fields
                    .get("signature")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    signature = Some(value.to_owned());
                    flush_thinking(&mut blocks, &mut text, &mut signature);
                }
            }
        }
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

fn flush_thinking(
    blocks: &mut Vec<ThinkingBlock>,
    text: &mut String,
    signature: &mut Option<String>,
) {
    if let Some(signature) = signature.take() {
        blocks.push(ThinkingBlock::Thinking {
            thinking: std::mem::take(text),
            signature,
        });
    } else {
        text.clear();
    }
}

pub fn get_combined_audio_content(
    chunks: &[Chunk],
    now: i64,
) -> Result<AudioResponse, AssemblyError> {
    let mut bytes = Vec::new();
    let mut transcript = String::new();
    let mut expires_at = None;
    let mut id = None;
    for chunk in chunks {
        for choice in &chunk.choices {
            let Some(audio) = choice.delta.as_ref().and_then(|delta| delta.audio.as_ref()) else {
                continue;
            };
            if let Some(Value::String(data)) = &audio.data {
                let fragment = decode_stream_audio_fragment(
                    data,
                    DEFAULT_AUDIO_LIMIT.saturating_sub(bytes.len()),
                )?;
                bytes.extend(fragment);
            }
            if let Some(Value::String(value)) = &audio.transcript {
                transcript.push_str(value);
            }
            if let Some(value) = &audio.expires_at {
                if !value.is_null() && (value.is_i64() || value.is_u64() || value.is_boolean()) {
                    expires_at = Some(value.clone());
                }
            }
            if let Some(Value::String(value)) = &audio.id {
                id = Some(value.clone());
            }
        }
    }
    let expires_at = expires_at
        .filter(|value| !value.as_bool().unwrap_or(false) && value != &Value::from(0))
        .unwrap_or_else(|| Value::from(now.saturating_add(3600)));
    Ok(AudioResponse {
        data: STANDARD.encode(bytes),
        expires_at,
        transcript,
        id,
    })
}

pub fn decode_stream_audio_fragment(
    fragment: &str,
    max_decoded_bytes: usize,
) -> Result<Vec<u8>, AssemblyError> {
    let filtered: String = fragment
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=')
        })
        .collect();
    let decoded = STANDARD
        .decode(filtered.as_bytes())
        .map_err(|error| AssemblyError::InvalidAudioBase64(error.to_string()))?;
    if decoded.len() > max_decoded_bytes {
        return Err(AssemblyError::AudioTooLarge {
            limit: max_decoded_bytes,
        });
    }
    Ok(decoded)
}

#[cfg(test)]
mod stream_content_assembly_cases;
