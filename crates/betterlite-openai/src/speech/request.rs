use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::speech::error::SpeechError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechFormat {
    Mp3,
    Opus,
    Aac,
    Flac,
    Wav,
    Pcm,
}

impl SpeechFormat {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
            Self::Aac => "aac",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Pcm => "pcm",
        }
    }
}

// The optional parameters LiteLLM hands audio.speech.create unchanged; extra_headers is an SDK kwarg, not a body field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeechOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<SpeechFormat>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub speed: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing)]
    pub extra_headers: IndexMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
pub struct SpeechRequest<'request> {
    pub model: &'request str,
    pub input: &'request str,
    pub voice: &'request str,
    pub options: &'request SpeechOptions,
}

// The body audio.speech.create serializes: the required keywords first, then only the options the caller set.
#[derive(Serialize)]
struct WireBody<'request> {
    input: &'request str,
    model: &'request str,
    voice: &'request str,
    #[serde(flatten)]
    options: &'request SpeechOptions,
}

pub fn wire_body(request: &SpeechRequest<'_>) -> Result<Vec<u8>, SpeechError> {
    serde_json::to_vec(&WireBody {
        input: request.input,
        model: request.model,
        voice: request.voice,
        options: request.options,
    })
    .map_err(|source| SpeechError::Encode { source })
}

// LiteLLM's speech_request_body: model and voice with every option except the caller headers the SDK takes as kwargs.
pub fn logged_body(request: &SpeechRequest<'_>) -> Result<IndexMap<String, Json>, SpeechError> {
    let mut body = IndexMap::from([
        (String::from("model"), Json::Text(request.model.to_owned())),
        (String::from("voice"), Json::Text(request.voice.to_owned())),
    ]);
    let options = request.options;
    if let Some(format) = options.response_format {
        body.insert(
            String::from("response_format"),
            Json::Text(format.wire_name().to_owned()),
        );
    }
    if let Some(speed) = options.speed {
        let number = Number::from_str(&speed.to_string())
            .map_err(|source| SpeechError::Encode { source })?;
        body.insert(String::from("speed"), Json::Number(number));
    }
    if let Some(instructions) = &options.instructions {
        body.insert(
            String::from("instructions"),
            Json::Text(instructions.clone()),
        );
    }
    Ok(body)
}
