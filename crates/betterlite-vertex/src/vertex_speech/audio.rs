use betterlite_prompts::arguments::json::Json;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

use super::error::{Base64Error, VertexSpeechError};

// LiteLLM builds its binary response with status_code=200 whatever the provider answered
const SYNTHESIS_STATUS: u16 = 200;

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
    let mut values: Vec<u8> = Vec::with_capacity(text.len());
    let mut trailing_pads: usize = 0;
    for byte in text.bytes() {
        if byte == b'=' {
            trailing_pads = trailing_pads.saturating_add(1);
            continue;
        }
        let Some(value) = sextet(byte) else {
            continue;
        };
        values.push(value);
        trailing_pads = 0;
    }
    let mut decoded = Vec::new();
    for group in values.chunks_exact(4) {
        let &[first, second, third, fourth] = group else {
            continue;
        };
        decoded.push((first << 2) | (second >> 4));
        decoded.push(second.wrapping_shl(4) | (third >> 2));
        decoded.push(third.wrapping_shl(6) | fourth);
    }
    match values.chunks_exact(4).remainder() {
        [] => {}
        [first, second] if trailing_pads >= 2 => {
            decoded.push((first << 2) | (second >> 4));
        }
        [first, second, third] if trailing_pads >= 1 => {
            decoded.push((first << 2) | (second >> 4));
            decoded.push(second.wrapping_shl(4) | (third >> 2));
        }
        [_] => return Err(Base64Error::SingleCharacter),
        [_first, _second] | [_first, _second, _third] => {
            return Err(Base64Error::MissingPadding);
        }
    }
    Ok(decoded)
}

fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => byte.checked_sub(b'A'),
        b'a'..=b'z' => byte.checked_sub(b'a')?.checked_add(26),
        b'0'..=b'9' => byte.checked_sub(b'0')?.checked_add(52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
