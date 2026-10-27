use betterlite_streaming::stream_content_assembly::{
    Chunk, ContentField, decode_stream_audio_fragment, get_combined_audio_content,
    get_combined_content, get_combined_thinking_content,
};
use serde::Deserialize;
use serde_json::json;

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
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
            Input::ContentAndReasoning { chunks } => {
                let content = get_combined_content(chunks, ContentField::Content);
                let reasoning = get_combined_content(chunks, ContentField::Reasoning);
                match (content, reasoning) {
                    (Ok(content), Ok(reasoning)) => {
                        json!({"content": content, "reasoning": reasoning})
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        json!({"ok": false, "error": error.to_string()})
                    }
                }
            }
            Input::ThinkingSignatureBoundaries { chunks } => {
                json!(get_combined_thinking_content(chunks))
            }
            Input::AudioFragmentsAndExpiry { chunks, now } => {
                match get_combined_audio_content(chunks, *now) {
                    Ok(audio) => json!(audio),
                    Err(error) => json!({"ok": false, "error": error.to_string()}),
                }
            }
            Input::AudioBase64Decoder {
                fragment,
                max_decoded_bytes,
            } => match decode_stream_audio_fragment(fragment, *max_decoded_bytes) {
                Ok(bytes) => json!({"ok": true, "bytes": bytes}),
                Err(error) => json!({"ok": false, "error": error.to_string()}),
            },
        };
        Ok(result)
    }
}
