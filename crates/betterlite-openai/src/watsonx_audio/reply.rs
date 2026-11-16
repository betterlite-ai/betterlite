use serde_json::{Map, Value};

use super::error::{WatsonxAudioError, body_text};
use crate::api::transcription_contract::TranscriptionResponse;

// ast-grep-ignore: no-long-names
pub fn decode_watsonx_transcription_reply(
    bytes: &[u8],
) -> Result<TranscriptionResponse, WatsonxAudioError> {
    let mut fields: Map<String, Value> = serde_json::from_slice(bytes)
        .map_err(|source| WatsonxAudioError::InvalidReply { source })?;
    fields.remove("model");
    let text = match fields.remove("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text),
        Some(_) => return Err(WatsonxAudioError::NotText { field: "text" }),
    };
    let usage_is_set = fields.get("usage").is_some_and(|usage| !usage.is_null());
    if text.is_none() && !usage_is_set {
        return Err(WatsonxAudioError::MissingFields {
            body: body_text(bytes),
        });
    }
    fields.entry(String::from("usage")).or_insert(Value::Null);
    Ok(TranscriptionResponse {
        text,
        extra: fields,
    })
}
