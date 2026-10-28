use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use std::mem;
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

#[derive(Debug, Deserialize)]
pub struct Chunk {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub delta: Option<Delta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<ChunkText>,
    #[serde(default)]
    pub reasoning_content: Option<ChunkText>,
    #[serde(default)]
    pub thinking_blocks: Option<ThinkingBlocks>,
    #[serde(default)]
    pub audio: Option<AudioDelta>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ChunkText {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ThinkingBlocks {
    List(Vec<ThinkingInput>),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
pub struct ThinkingInput {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AudioDelta {
    #[serde(default)]
    pub data: Option<ChunkText>,
    #[serde(default)]
    pub transcript: Option<ChunkText>,
    #[serde(default)]
    pub expires_at: Option<ExpiryValue>,
    #[serde(default)]
    pub id: Option<ChunkText>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
#[serde(untagged)]
pub enum ExpiryValue {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ThinkingBlock {
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    #[serde(rename = "redacted_thinking")]
    Redacted { data: String },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AudioResponse {
    pub data: String,
    pub expires_at: ExpiryValue,
    pub transcript: String,
    pub id: Option<String>,
}

pub fn get_combined_content(
    chunks: &[Chunk],
    field: ContentField,
) -> Result<String, AssemblyError> {
    let mut combined = String::new();
    for_each_delta(chunks, |delta| append_content(&mut combined, delta, field))?;
    Ok(combined)
}

fn for_each_delta<F>(chunks: &[Chunk], mut action: F) -> Result<(), AssemblyError>
where
    F: FnMut(Option<&Delta>) -> Result<(), AssemblyError>,
{
    for chunk in chunks {
        for choice in &chunk.choices {
            action(choice.delta.as_ref())?;
        }
    }
    Ok(())
}

fn append_content(
    combined: &mut String,
    delta: Option<&Delta>,
    field: ContentField,
) -> Result<(), AssemblyError> {
    let value = delta.and_then(|chunk_delta| match field {
        ContentField::Content => chunk_delta.content.as_ref(),
        ContentField::Reasoning => chunk_delta.reasoning_content.as_ref(),
    });
    match value {
        Some(ChunkText::Text(text)) => combined.push_str(text),
        Some(ChunkText::Other(_)) => return Err(AssemblyError::NonStringContent),
        None => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentField {
    Content,
    Reasoning,
}

#[must_use]
pub fn get_combined_thinking_content(chunks: &[Chunk]) -> Option<Vec<ThinkingBlock>> {
    let mut state = ThinkingState::default();
    for chunk in chunks {
        for choice in &chunk.choices {
            let Some(Some(ThinkingBlocks::List(blocks))) = choice
                .delta
                .as_ref()
                .map(|delta| delta.thinking_blocks.as_ref())
            else {
                continue;
            };
            for block in blocks {
                state.consume(block);
            }
        }
    }
    state.finish()
}

#[derive(Default)]
struct ThinkingState {
    blocks: Vec<ThinkingBlock>,
    text: String,
    signature: Option<String>,
}

impl ThinkingState {
    fn consume(&mut self, block: &ThinkingInput) {
        if block.r#type.as_deref() == Some("redacted_thinking") {
            self.flush();
            if let Some(data) = block.data.as_deref().filter(|data| !data.is_empty()) {
                self.blocks.push(ThinkingBlock::Redacted {
                    data: data.to_owned(),
                });
            }
            return;
        }
        if let Some(thinking) = block.thinking.as_deref().filter(|text| !text.is_empty()) {
            self.text.push_str(thinking);
        }
        if let Some(signature) = block.signature.as_deref().filter(|value| !value.is_empty()) {
            self.signature = Some(signature.to_owned());
            self.flush();
        }
    }

    fn flush(&mut self) {
        if let Some(signature_value) = self.signature.take() {
            self.blocks.push(ThinkingBlock::Thinking {
                thinking: mem::take(&mut self.text),
                signature: signature_value,
            });
        } else {
            self.text.clear();
        }
    }

    fn finish(self) -> Option<Vec<ThinkingBlock>> {
        if self.blocks.is_empty() {
            None
        } else {
            Some(self.blocks)
        }
    }
}

pub fn get_combined_audio_content(
    chunks: &[Chunk],
    now: i64,
) -> Result<AudioResponse, AssemblyError> {
    let mut state = AudioState::default();
    for_each_delta(chunks, |delta| state.consume(delta))?;
    Ok(state.finish(now))
}

#[derive(Default)]
struct AudioState {
    bytes: Vec<u8>,
    transcript: String,
    expires_at: Option<ExpiryValue>,
    id: Option<String>,
}

impl AudioState {
    fn consume(&mut self, delta: Option<&Delta>) -> Result<(), AssemblyError> {
        let Some(audio) = delta.and_then(|chunk_delta| chunk_delta.audio.as_ref()) else {
            return Ok(());
        };
        if let Some(ChunkText::Text(data)) = &audio.data {
            let remaining = DEFAULT_AUDIO_LIMIT.saturating_sub(self.bytes.len());
            self.bytes
                .extend(decode_stream_audio_fragment(data, remaining)?);
        }
        if let Some(ChunkText::Text(value)) = &audio.transcript {
            self.transcript.push_str(value);
        }
        if let Some(value) = &audio.expires_at {
            self.expires_at = Some(value.clone());
        }
        if let Some(ChunkText::Text(value)) = &audio.id {
            self.id = Some(value.clone());
        }
        Ok(())
    }

    fn finish(self, now: i64) -> AudioResponse {
        let expires_at = self
            .expires_at
            .filter(|value| !value.is_falsey())
            .unwrap_or_else(|| ExpiryValue::Signed(now.saturating_add(3600)));
        AudioResponse {
            data: STANDARD.encode(self.bytes),
            expires_at,
            transcript: self.transcript,
            id: self.id,
        }
    }
}

impl ExpiryValue {
    const fn is_falsey(&self) -> bool {
        let is_falsey = matches!(
            self,
            Self::Boolean(false) | Self::Signed(0) | Self::Unsigned(0)
        );
        is_falsey
    }
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

#[path = "stream_content_assembly/tests.rs"]
mod stream_content_assembly_cases;
