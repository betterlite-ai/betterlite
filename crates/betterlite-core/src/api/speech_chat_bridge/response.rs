use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::speech_chat_bridge::audio_base64::decode_chat_audio_base64;
use betterlite_openai::files::records::BinaryContent;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::SpeechCompletionAnswer;
use super::error::SpeechTransformError;
use super::{GEMINI_TTS_RAW_RESPONSE_FORMAT, is_gemini_tts_model};

// LiteLLM's HttpxBinaryResponseContent plus the response cost its hidden params carry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechBinaryResponse {
    pub content: BinaryContent,
    pub response_cost: Option<Decimal>,
}

// the fields of LiteLLM's ChatCompletionAudio that the transformation reads
#[derive(Debug, Deserialize)]
struct ChatAudioPart {
    data: String,
}

const RIFF_SIZE_BASE: u32 = 36;
const WAV_SAMPLE_RATE: u32 = 24_000;
const WAV_CHANNELS: u16 = 1;
const WAV_BYTE_RATE: u32 = 48_000;
const WAV_BLOCK_ALIGN: u16 = 2;
const WAV_BITS_PER_SAMPLE: u16 = 16;
const WAV_PCM_FORMAT: u16 = 1;
const WAV_HEADER_BYTES: usize = 44;
const CONTENT_TYPE_HEADER: &str = "content-type";

// the response side of LiteLLM's transform_response: choices[0].message.audio decides the bytes and the response model the conversion
pub fn transform_response(
    answer: SpeechCompletionAnswer,
    response_format: Option<&str>,
) -> Result<SpeechBinaryResponse, SpeechTransformError> {
    let SpeechCompletionAnswer {
        response,
        hidden_params,
    } = answer;
    let Some(choice) = response.choices.first() else {
        return Err(SpeechTransformError::NoChoices);
    };
    let Some(audio) = choice.message.audio.clone() else {
        return Err(SpeechTransformError::MissingAudio);
    };
    let part: ChatAudioPart = serde_json::from_value(audio)
        .map_err(|source| SpeechTransformError::AudioPart { source })?;
    let decoded = decode_chat_audio_base64(&part.data).map_err(SpeechTransformError::Audio)?;
    let (body, content_type) = if is_gemini_tts_model(&response.model) {
        gemini_tts_response_body(decoded, response_format)?
    } else {
        (decoded, "audio/mpeg")
    };
    let headers = IndexMap::from([(
        String::from(CONTENT_TYPE_HEADER),
        String::from(content_type),
    )]);
    Ok(SpeechBinaryResponse {
        content: BinaryContent {
            status: 200,
            headers,
            body,
        },
        response_cost: completion_cost(&hidden_params),
    })
}

// Gemini returns raw PCM16: pcm keeps it, every other accepted format becomes a 24 kHz mono 16-bit WAV
fn gemini_tts_response_body(
    decoded: Vec<u8>,
    response_format: Option<&str>,
) -> Result<(Vec<u8>, &'static str), SpeechTransformError> {
    if response_format == Some(GEMINI_TTS_RAW_RESPONSE_FORMAT) {
        return Ok((decoded, "audio/pcm"));
    }
    Ok((pcm16_to_wav(&decoded)?, "audio/wav"))
}

// struct.pack("<4sI4s4sIHHIIHH4sI") over the fixed header, then the raw PCM16 data with no padding for an odd length
fn pcm16_to_wav(pcm: &[u8]) -> Result<Vec<u8>, SpeechTransformError> {
    let data_size = u32::try_from(pcm.len()).map_err(|_unfit| SpeechTransformError::WavTooLarge)?;
    let riff_size = RIFF_SIZE_BASE
        .checked_add(data_size)
        .ok_or(SpeechTransformError::WavTooLarge)?;
    let mut wav = Vec::with_capacity(pcm.len().saturating_add(WAV_HEADER_BYTES));
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&WAV_PCM_FORMAT.to_le_bytes());
    wav.extend_from_slice(&WAV_CHANNELS.to_le_bytes());
    wav.extend_from_slice(&WAV_SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&WAV_BYTE_RATE.to_le_bytes());
    wav.extend_from_slice(&WAV_BLOCK_ALIGN.to_le_bytes());
    wav.extend_from_slice(&WAV_BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

// LiteLLM keeps the hidden response cost only when it is a Python float; ints, booleans, strings and missing values drop it
#[must_use]
pub fn completion_cost(hidden_params: &HiddenParams) -> Option<Decimal> {
    match hidden_params.get("response_cost") {
        Some(Json::Number(number)) if number.is_f64() => {
            number.as_f64().and_then(Decimal::from_f64_retain)
        }
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Number(_),
        )
        | None => None,
    }
}
