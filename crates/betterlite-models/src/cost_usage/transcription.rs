use serde::Deserialize;
use serde::de::{self, Deserializer};

use super::{CostUsage, PromptDetails};

/// A caller-owned, already-typed transcription usage object.
///
/// Recognition is the Rust type system: a raw dictionary that merely carries
/// `type: "tokens"` is never this type. The `Tokens` payload is boxed so its size
/// does not dwarf the `Duration` variant's.
#[derive(Debug, Clone)]
pub enum TranscriptionUsage {
    Duration { seconds: f64 },
    Tokens(Box<TranscriptionTokens>),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TranscriptionUsageKind {
    Duration,
    Tokens,
}

// A flat intermediate shape, not `#[serde(tag = "type")]` directly on the public enum:
// serde's internally tagged enum support buffers each variant's fields through its own
// `Content` type before matching the tag, and that buffer does not round-trip a
// `serde_json` `arbitrary_precision` number back into a bare `f64` field, failing with
// "invalid type: map, expected f64". A plain struct field needs no such buffering.
#[derive(Debug, Clone, Deserialize)]
struct RawTranscriptionUsage {
    #[serde(rename = "type")]
    kind: TranscriptionUsageKind,
    seconds: Option<f64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    input_token_details: Option<TranscriptionTokenDetails>,
}

impl<'de> Deserialize<'de> for TranscriptionUsage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawTranscriptionUsage::deserialize(deserializer)?;
        match raw.kind {
            TranscriptionUsageKind::Duration => {
                let seconds = raw
                    .seconds
                    .ok_or_else(|| de::Error::missing_field("seconds"))?;
                Ok(Self::Duration { seconds })
            }
            TranscriptionUsageKind::Tokens => {
                let input_tokens = raw
                    .input_tokens
                    .ok_or_else(|| de::Error::missing_field("input_tokens"))?;
                let output_tokens = raw
                    .output_tokens
                    .ok_or_else(|| de::Error::missing_field("output_tokens"))?;
                let total_tokens = raw
                    .total_tokens
                    .ok_or_else(|| de::Error::missing_field("total_tokens"))?;
                let input_token_details = raw
                    .input_token_details
                    .ok_or_else(|| de::Error::missing_field("input_token_details"))?;
                Ok(Self::Tokens(Box::new(TranscriptionTokens {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    input_token_details,
                })))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TranscriptionTokens {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: TranscriptionTokenDetails,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TranscriptionTokenDetails {
    pub audio_tokens: i64,
    pub text_tokens: i64,
}

/// Duration usage carries no token counts, so it transforms to `None` and the caller
/// keeps billing it by duration. Token usage copies its counts without recomputing them.
#[must_use]
pub fn transform_transcription_usage(usage: &TranscriptionUsage) -> Option<CostUsage> {
    let TranscriptionUsage::Tokens(tokens) = usage else {
        return None;
    };
    Some(CostUsage {
        prompt_tokens: tokens.input_tokens,
        completion_tokens: tokens.output_tokens,
        total_tokens: tokens.total_tokens,
        prompt_details: Some(PromptDetails {
            text_tokens: Some(tokens.input_token_details.text_tokens),
            audio_tokens: Some(tokens.input_token_details.audio_tokens),
            ..PromptDetails::default()
        }),
        completion_details: None,
        cache_read_tokens: None,
    })
}

mod tests;
