use serde_json::{Map, Value};

use crate::api::transcription_contract::TranscriptionResponse;
use crate::openai_base_config::{Headers, Object};
use crate::ovhcloud_chat::error::OvhCloudError;

use super::MAX_REPLY_BYTES;

// LiteLLM keeps the whole reply body as hidden params beside the response it builds from it
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub response: TranscriptionResponse,
    pub hidden: Object,
}

// LiteLLM reads text, else transcript, else the empty string; a present but empty string falls through as falsy does
fn first_text(fields: &Map<String, Value>) -> String {
    for name in ["text", "transcript"] {
        let text = fields.get(name).and_then(Value::as_str);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return text.to_owned();
        }
    }
    String::new()
}

// seconds wins when the key is present and not null, even when it is zero; the value lands on duration in the kept body
fn normalized_duration(fields: &Map<String, Value>) -> Option<Value> {
    match fields.get("seconds") {
        Some(value) if !value.is_null() => Some(value.clone()),
        _ => fields
            .get("duration")
            .filter(|value| !value.is_null())
            .cloned(),
    }
}

// a body that is not a JSON object gives the exception with the body text, the upstream status and the headers
pub fn decode_transcription_reply(
    body: &[u8],
    status: u16,
    headers: &Headers,
) -> Result<Transcription, OvhCloudError> {
    if body.len() > MAX_REPLY_BYTES {
        return Err(OvhCloudError {
            message: format!(
                "the OVHCloud transcription reply is {} bytes, over the {MAX_REPLY_BYTES} byte limit",
                body.len()
            ),
            status_code: None,
            headers: Headers::new(),
        });
    }
    let mut fields = serde_json::from_slice::<Object>(body).map_err(|_error| OvhCloudError {
        message: String::from_utf8_lossy(body).into_owned(),
        status_code: Some(status),
        headers: headers.clone(),
    })?;
    let text = first_text(&fields);
    if let Some(duration) = normalized_duration(&fields) {
        fields.insert(String::from("duration"), duration);
    }
    Ok(Transcription {
        response: TranscriptionResponse {
            text: Some(text),
            extra: Object::new(),
        },
        hidden: fields,
    })
}
