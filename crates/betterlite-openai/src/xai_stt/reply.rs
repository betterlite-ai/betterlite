use betterlite_http::proxy::UpstreamResponse;

use crate::api::batch_records::WireDecodeLimits;
use crate::files::stream::flatten_headers;

use super::error::XaiSttError;
use super::records::{
    SttMetadata, SttPayload, SttWord, TranscriptionResponse, TranscriptionTask, TranscriptionWord,
};

// LiteLLM raises before parsing on a non-2xx reply, and a parse failure keeps the status it was handed
pub fn decode_xai_stt_reply(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
) -> Result<TranscriptionResponse, XaiSttError> {
    let headers = flatten_headers(&reply.headers);
    if reply.status >= 400 {
        return Err(XaiSttError::Status {
            status: reply.status,
            message: body_text(&reply.body),
            headers,
        });
    }
    if reply.body.len() > limits.max_json_bytes {
        return Err(XaiSttError::TooLarge {
            status: reply.status,
            headers,
            limit: limits.max_json_bytes,
        });
    }
    let payload: SttPayload =
        serde_json::from_slice(&reply.body).map_err(|source| XaiSttError::Decode {
            status: reply.status,
            headers,
            source,
        })?;
    Ok(project(payload))
}

fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_invalid| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

fn project(payload: SttPayload) -> TranscriptionResponse {
    let duration = payload.duration;
    let words = payload
        .words
        .as_ref()
        .map(|words| words.iter().map(to_word).collect());
    TranscriptionResponse {
        text: payload.text.clone(),
        task: TranscriptionTask::Transcribe,
        language: payload.language.clone(),
        duration,
        words,
        metadata: SttMetadata {
            text: payload.text,
            language: payload.language,
            duration,
            words: payload.words,
            native_fields: payload.native_fields,
            audio_transcription_duration: duration,
        },
    }
}

fn to_word(word: &SttWord) -> TranscriptionWord {
    TranscriptionWord {
        word: word.text.clone(),
        start: word.start,
        end: word.end,
        speaker: word.speaker,
    }
}
