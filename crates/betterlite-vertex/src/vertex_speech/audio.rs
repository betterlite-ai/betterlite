use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::general_purpose::{GeneralPurpose, NO_PAD};
use betterlite_prompts::arguments::json::Json;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

use super::error::{Base64Error, VertexSpeechError};

// LiteLLM builds its binary response with status_code=200 whatever the provider answered
const SYNTHESIS_STATUS: u16 = 200;

const LENIENT: GeneralPurpose =
    GeneralPurpose::new(&STANDARD, NO_PAD.with_decode_allow_trailing_bits(true));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryAudio {
    pub status: u16,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

pub fn decode_vertex_speech_audio(
    body: &[u8],
    sniff: &dyn Fn(&[u8]) -> Option<String>,
) -> Result<BinaryAudio, VertexSpeechError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(VertexSpeechError::ResponseTooLarge {
            size: body.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let Json::Object(fields) =
        serde_json::from_slice::<Json>(body).map_err(VertexSpeechError::ResponseJson)?
    else {
        return Err(VertexSpeechError::ResponseNotObject);
    };
    let Some(content) = fields.get("audioContent") else {
        return Err(VertexSpeechError::MissingAudioContent);
    };
    if !content.is_truthy() {
        return Err(VertexSpeechError::MissingAudioContent);
    }
    let Json::Text(text) = content else {
        return Err(VertexSpeechError::AudioContentNotText);
    };
    if !text.is_ascii() {
        return Err(VertexSpeechError::AudioContentNotAscii);
    }
    let bytes = decode_base64(text).map_err(VertexSpeechError::Base64)?;
    Ok(BinaryAudio {
        status: SYNTHESIS_STATUS,
        content_type: sniff(&bytes),
        bytes,
    })
}

// CPython's base64.b64decode skips bytes off the alphabet, rejects a lone final character and needs the last group's padding
fn decode_base64(text: &str) -> Result<Vec<u8>, Base64Error> {
    let is_symbol = |byte: &u8| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/');
    let symbols: Vec<u8> = text.bytes().filter(is_symbol).collect();
    let pads = text
        .bytes()
        .rev()
        .take_while(|byte| !is_symbol(byte))
        .filter(|byte| *byte == b'=')
        .count();
    if matches!((symbols.len() % 4, pads), (2, 0 | 1) | (3, 0)) {
        return Err(Base64Error::MissingPadding);
    }
    // only a lone final symbol is left for the engine to reject
    LENIENT
        .decode(symbols)
        .map_err(|_error| Base64Error::SingleCharacter)
}
