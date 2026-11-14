use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

use super::error::VertexSpeechError;

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
    let bytes =
        decode_stream_audio_fragment(text, MAX_RESPONSE_BYTES).map_err(VertexSpeechError::Audio)?;
    Ok(BinaryAudio {
        status: SYNTHESIS_STATUS,
        content_type: sniff(&bytes),
        bytes,
    })
}
