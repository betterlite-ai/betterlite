use betterlite_streaming::stream_content_assembly::wire::Chunk;
use betterlite_streaming::stream_content_assembly::{
    AssemblyError, ContentField, decode_stream_audio_fragment, get_combined_audio_content,
    get_combined_content, get_combined_thinking_content,
};
use serde::Deserialize;
use serde_json::json;

#[path = "stream_content_assembly/tests.rs"]
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Batch {
        cases: Vec<Self>,
    },
    ContentAndReasoning {
        chunks: Vec<Chunk>,
    },
    ThinkingSignatureBoundaries {
        chunks: Vec<Chunk>,
    },
    AudioFragmentsAndExpiry {
        chunks: Vec<Chunk>,
        now: i64,
    },
    AudioBase64Decoder {
        fragment: String,
        max_decoded_bytes: usize,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::Batch { cases } => {
                let results: Result<Vec<_>, _> = cases.iter().map(Self::run).collect();
                json!(results?)
            }
            Input::ContentAndReasoning { chunks } => {
                let content = get_combined_content(chunks, ContentField::Content);
                let reasoning = get_combined_content(chunks, ContentField::Reasoning);
                match (content, reasoning) {
                    (Ok(content_value), Ok(reasoning_value)) => {
                        json!({"content": content_value, "reasoning": reasoning_value})
                    }
                    (Err(error), _) | (_, Err(error)) => error_output(&error),
                }
            }
            Input::ThinkingSignatureBoundaries { chunks } => {
                match get_combined_thinking_content(chunks) {
                    Ok(blocks) => json!(blocks),
                    Err(error) => error_output(&error),
                }
            }
            Input::AudioFragmentsAndExpiry { chunks, now } => {
                match get_combined_audio_content(chunks, *now) {
                    Ok(audio) => json!(audio),
                    Err(error) => error_output(&error),
                }
            }
            Input::AudioBase64Decoder {
                fragment,
                max_decoded_bytes,
            } => match decode_stream_audio_fragment(fragment, *max_decoded_bytes) {
                Ok(bytes) => json!({"ok": true, "bytes": bytes}),
                Err(error) => error_output(&error),
            },
        };
        Ok(result)
    }
}

fn error_output(error: &AssemblyError) -> serde_json::Value {
    let error_type = match error {
        AssemblyError::NonStringContent => "TypeError",
        AssemblyError::NonAsciiAudio => "ValueError",
        AssemblyError::InvalidAudioBase64 { .. } | AssemblyError::InvalidAudioPadding => "Error",
        AssemblyError::AudioTooLarge { .. } | AssemblyError::OutputTooLarge => "OutputTooLarge",
    };
    json!({"ok": false, "error_type": error_type, "error": error.to_string()})
}
