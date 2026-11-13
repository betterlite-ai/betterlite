use serde_json::{Map, Value};

use crate::api::transcription_contract::TranscriptionResponse;
use crate::openai_base_config::{Headers, Object};

use super::error::ScalewayAudioError;

// LiteLLM keeps the whole reply body as hidden params beside the response it builds from it
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub response: TranscriptionResponse,
    pub hidden: Object,
}

// httpx reads the header case-insensitively, so a caller's Content-Type spelling still picks the JSON branch
fn content_type(headers: &Headers) -> String {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map_or_else(String::new, |(_, value)| value.to_lowercase())
}

fn json_reply(fields: Map<String, Value>) -> Transcription {
    let text = fields
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut extra = Object::new();
    for name in ["segments", "language"] {
        if let Some(value) = fields.get(name) {
            extra.insert(String::from(name), value.clone());
        }
    }
    Transcription {
        response: TranscriptionResponse {
            text: Some(text),
            extra,
        },
        hidden: fields,
    }
}

// a reply that is not labelled JSON is handed back as text, and the JSON branch's parse failure carries the body text
pub fn decode_transcription_reply(
    body: &[u8],
    status: u16,
    headers: &Headers,
) -> Result<Transcription, ScalewayAudioError> {
    if !content_type(headers).contains("application/json") {
        return Ok(Transcription {
            response: TranscriptionResponse {
                text: Some(String::from_utf8_lossy(body).into_owned()),
                extra: Object::new(),
            },
            hidden: Object::new(),
        });
    }
    let fields = serde_json::from_slice::<Object>(body).map_err(|_error| ScalewayAudioError {
        message: String::from_utf8_lossy(body).into_owned(),
        status_code: status,
        headers: headers.clone(),
    })?;
    Ok(json_reply(fields))
}
