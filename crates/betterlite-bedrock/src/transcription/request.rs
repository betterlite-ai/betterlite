use betterlite_models::decisions::Json;
use serde::Serialize;

use super::error::TranscriptionError;
use super::params::{Params, text_param};

pub const SUPPORTED_PARAMS: [&str; 4] = ["language", "prompt", "temperature", "response_format"];

const INSTRUCTION: &str = "Transcribe the audio. Respond with only the transcript.";
const SYSTEM_PROMPT: &str = "You are a transcription assistant.";
const MAX_TOKENS: u32 = 4096;

#[derive(Serialize)]
struct Body<'body> {
    messages: [Message<'body>; 1],
    system: [TextBlock<'body>; 1],
    #[serde(rename = "inferenceConfig")]
    inference_config: InferenceConfig<'body>,
}

#[derive(Serialize)]
struct Message<'body> {
    role: &'static str,
    content: (AudioBlock<'body>, TextBlock<'body>),
}

#[derive(Serialize)]
struct AudioBlock<'body> {
    audio: Audio<'body>,
}

#[derive(Serialize)]
struct Audio<'body> {
    format: &'body str,
    source: Source<'body>,
}

#[derive(Serialize)]
struct Source<'body> {
    bytes: &'body str,
}

#[derive(Serialize)]
struct TextBlock<'body> {
    text: &'body str,
}

#[derive(Serialize)]
struct InferenceConfig<'body> {
    #[serde(rename = "maxTokens")]
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<&'body Json>,
}

fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::Text(_) => "string",
        Json::List(_) => "array",
        Json::Object(_) => "object",
    }
}

fn audio_fields(audio: &Json) -> Result<(&str, &str), TranscriptionError> {
    let Json::Object(fields) = audio else {
        return Err(TranscriptionError::InvalidType {
            expected: "object",
            actual: type_name(audio),
        });
    };
    let Some(data) = text_param(fields, "data").filter(|data| !data.is_empty()) else {
        return Err(TranscriptionError::MissingField("audio.data"));
    };
    match text_param(fields, "format") {
        Some(format @ ("wav" | "mp3" | "flac" | "ogg")) => Ok((data, format)),
        Some(_) | None => Err(TranscriptionError::InvalidRequest(
            "audio.format must be wav, mp3, flac, or ogg",
        )),
    }
}

fn instruction(params: &Params) -> String {
    let language = text_param(params, "language")
        .filter(|language| !language.is_empty())
        .map(|language| format!(" The audio language is {language}."));
    let prompt = text_param(params, "prompt")
        .filter(|prompt| !prompt.is_empty())
        .map(|prompt| format!(" Additional context: {prompt}"));
    [Some(INSTRUCTION.to_owned()), language, prompt]
        .into_iter()
        .flatten()
        .collect()
}

pub fn transcription_body(audio: &Json, params: &Params) -> Result<String, TranscriptionError> {
    let (data, format) = audio_fields(audio)?;
    let text = instruction(params);
    let body = Body {
        messages: [Message {
            role: "user",
            content: (
                AudioBlock {
                    audio: Audio {
                        format,
                        source: Source { bytes: data },
                    },
                },
                TextBlock { text: &text },
            ),
        }],
        system: [TextBlock {
            text: SYSTEM_PROMPT,
        }],
        inference_config: InferenceConfig {
            max_tokens: MAX_TOKENS,
            temperature: params.get("temperature"),
        },
    };
    serde_json::to_string(&body).map_err(TranscriptionError::Encode)
}
