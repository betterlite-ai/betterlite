use serde_json::Number;

use crate::api::realtime_contracts::events::{
    AudioTranscriptionUsage, DurationUsage, InputTokenDetails, TokenUsage,
};
use crate::api::transcription_protocol::usage::TranscriptionUsage;

// LiteLLM hands the transformer's usage to the realtime close callback unchanged
#[must_use]
pub(super) fn audio_transcription_usage(
    usage: TranscriptionUsage,
) -> Option<AudioTranscriptionUsage> {
    match usage {
        TranscriptionUsage::Duration(duration) => Number::from_f64(duration.seconds)
            .map(|seconds| AudioTranscriptionUsage::Duration(DurationUsage { seconds })),
        TranscriptionUsage::Tokens(tokens) => {
            Some(AudioTranscriptionUsage::Tokens(Box::new(TokenUsage {
                input_tokens: tokens.input_tokens,
                output_tokens: tokens.output_tokens,
                total_tokens: tokens.total_tokens,
                input_token_details: InputTokenDetails {
                    text_tokens: tokens.input_token_details.text_tokens,
                    audio_tokens: tokens.input_token_details.audio_tokens,
                },
            })))
        }
    }
}
