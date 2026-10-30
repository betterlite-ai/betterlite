use base64::Engine as _;
use base64::alphabet;
use base64::engine::GeneralPurpose;
use base64::engine::general_purpose::{NO_PAD, STANDARD};
use std::mem;
use thiserror::Error;

pub mod wire;

use wire::{
    AudioResponse, Chunk, ChunkText, Delta, ExpiryInput, ExpiryValue, ThinkingBlock,
    ThinkingBlocks, ThinkingInput,
};

const DEFAULT_AUDIO_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AssemblyError {
    #[error("content field is not a string")]
    NonStringContent,
    #[error("invalid audio base64: {source}")]
    InvalidAudioBase64 {
        #[source]
        source: base64::DecodeError,
    },
    #[error("audio base64 requires ASCII input")]
    NonAsciiAudio,
    #[error("invalid audio base64 padding")]
    InvalidAudioPadding,
    #[error("decoded audio exceeds {limit} bytes")]
    AudioTooLarge { limit: usize },
    #[error("assembled output exceeds {DEFAULT_AUDIO_LIMIT} bytes")]
    OutputTooLarge,
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
        Some(ChunkText::Text(text)) => append_text(combined, text)?,
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

pub fn get_combined_thinking_content(
    chunks: &[Chunk],
) -> Result<Option<Vec<ThinkingBlock>>, AssemblyError> {
    let mut blocks = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .filter_map(|choice| choice.delta.as_ref())
        .filter_map(|delta| match delta.thinking_blocks.as_ref() {
            Some(ThinkingBlocks::List(items)) => Some(items),
            Some(ThinkingBlocks::Other(_)) | None => None,
        })
        .flatten();
    blocks
        .try_fold(ThinkingState::default(), |mut state, block| {
            state.consume(block)?;
            Ok(state)
        })
        .map(ThinkingState::finish)
}

#[derive(Default)]
struct ThinkingState {
    blocks: Vec<ThinkingBlock>,
    text: String,
    signature: Option<String>,
    size: usize,
}

impl ThinkingState {
    fn consume(&mut self, block: &ThinkingInput) -> Result<(), AssemblyError> {
        if block.r#type.as_deref() == Some("redacted_thinking") {
            self.flush();
            if let Some(data) = block.data.as_deref().filter(|data| !data.is_empty()) {
                self.size = checked_size(self.size, data.len())?;
                self.blocks.push(ThinkingBlock::Redacted {
                    data: data.to_owned(),
                });
            }
            return Ok(());
        }
        if let Some(thinking) = block.thinking.as_deref().filter(|text| !text.is_empty()) {
            self.size = checked_size(self.size, thinking.len())?;
            self.text.push_str(thinking);
        }
        if let Some(signature) = block.signature.as_deref().filter(|value| !value.is_empty()) {
            self.size = checked_size(self.size, signature.len())?;
            self.signature = Some(signature.to_owned());
            self.flush();
        }
        Ok(())
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
            append_text(&mut self.transcript, value)?;
        }
        if let Some(ExpiryInput::Integer(value)) = &audio.expires_at {
            self.expires_at = Some(match value {
                ExpiryValue::Boolean(boolean) => ExpiryValue::Signed(i64::from(*boolean)),
                ExpiryValue::Signed(_) | ExpiryValue::Unsigned(_) => value.clone(),
            });
        }
        if let Some(ChunkText::Text(value)) = &audio.id {
            checked_size(0, value.len())?;
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
    let filtered = decoder::normalize(fragment, max_decoded_bytes)?;
    let engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        NO_PAD.with_decode_allow_trailing_bits(true),
    );
    engine
        .decode(filtered)
        .map_err(|source| AssemblyError::InvalidAudioBase64 { source })
}

const fn checked_size(current: usize, additional: usize) -> Result<usize, AssemblyError> {
    match current.checked_add(additional) {
        Some(size) if size <= DEFAULT_AUDIO_LIMIT => Ok(size),
        _ => Err(AssemblyError::OutputTooLarge),
    }
}

fn append_text(target: &mut String, value: &str) -> Result<(), AssemblyError> {
    checked_size(target.len(), value.len())?;
    target.push_str(value);
    Ok(())
}

mod decoder;

mod tests;

#[path = "stream_content_assembly/edge/tests.rs"]
mod edge_tests;
