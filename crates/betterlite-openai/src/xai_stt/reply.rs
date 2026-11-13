use betterlite_http::proxy::UpstreamResponse;

use crate::api::batch_records::WireDecodeLimits;
use crate::files::stream::flatten_headers;

use super::error::XaiSttError;
use super::records::{
    SttMetadata, SttPayload, TranscriptionResponse, TranscriptionTask, TranscriptionWord,
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
            message: str::from_utf8(&reply.body).map_or_else(
                |_invalid| format!("{} bytes that are not UTF-8", reply.body.len()),
                str::to_owned,
            ),
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

fn project(payload: SttPayload) -> TranscriptionResponse {
    let words = payload.words.as_ref().map(|words| {
        words
            .iter()
            .map(|word| TranscriptionWord {
                word: word.text.clone(),
                start: word.start,
                end: word.end,
                speaker: word.speaker,
            })
            .collect()
    });
    TranscriptionResponse {
        text: payload.text.clone(),
        task: TranscriptionTask::Transcribe,
        language: payload.language.clone(),
        duration: payload.duration,
        words,
        metadata: SttMetadata {
            audio_transcription_duration: payload.duration,
            native: payload,
        },
    }
}
