use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::error::HttpError;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use serde_json::Value;

use crate::files::stream::flatten_headers;
use crate::openai_base_config::Object;
use crate::speech::result::SpeechResponse;

use super::error::MinimaxSpeechError;

const SUCCESS_STATUS: u16 = 200;
const FAILURE_STATUS: u16 = 500;

pub struct SpeechReply<'reply> {
    pub status: u16,
    pub headers: &'reply HeaderMap,
    pub body: &'reply [u8],
}

pub fn decode_speech_reply(reply: SpeechReply<'_>) -> Result<SpeechResponse, MinimaxSpeechError> {
    let parsed: Value = serde_json::from_slice(reply.body)
        .map_err(|source| MinimaxSpeechError::Decode { source })?;
    let Value::Object(fields) = parsed else {
        return Err(MinimaxSpeechError::NotObject);
    };
    check_status(&fields, reply.status)?;
    check_audio_url(&fields)?;
    let audio = audio_text(&fields)?;
    if audio.is_empty() {
        return Err(no_audio(&fields));
    }
    let bytes = decode_audio(&audio)?;
    Ok(binary_response(reply.headers, bytes))
}

fn check_status(fields: &Object, raw_status: u16) -> Result<(), MinimaxSpeechError> {
    let Some(status) = fields.get("status") else {
        return Ok(());
    };
    if status.is_null() {
        return Ok(());
    }
    let Value::Number(number) = status else {
        return Err(MinimaxSpeechError::Field { field: "status" });
    };
    if number.as_f64() == Some(0.0) {
        return Ok(());
    }
    let detail = match fields.get("ced") {
        None => String::from("Unknown error"),
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(Value::Null | Value::String(_)) => format!("API returned status {number}"),
        Some(_) => return Err(MinimaxSpeechError::Field { field: "ced" }),
    };
    Err(MinimaxSpeechError::Reply {
        status: raw_status,
        message: format!("MiniMax TTS error: {detail}"),
    })
}

fn data_object(fields: &Object) -> Result<Option<&Object>, MinimaxSpeechError> {
    match fields.get("data") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(data)) => Ok(Some(data)),
        Some(_) => Err(MinimaxSpeechError::Field { field: "data" }),
    }
}

fn check_audio_url(fields: &Object) -> Result<(), MinimaxSpeechError> {
    let Some(data) = data_object(fields)? else {
        return Ok(());
    };
    match data.get("audio_url") {
        Some(Value::String(url)) if !url.is_empty() => Err(MinimaxSpeechError::Reply {
            status: FAILURE_STATUS,
            message: format!(
                "URL output format is not yet supported. Use 'hex' format or fetch from URL: {url}"
            ),
        }),
        Some(Value::String(_) | Value::Null) | None => Ok(()),
        Some(_) => Err(MinimaxSpeechError::Reply {
            status: FAILURE_STATUS,
            message: String::from("URL output format is not yet supported. Use 'hex' format"),
        }),
    }
}

// data.audio comes first and the older top-level audio_file only when data.audio is absent or null
fn audio_text(fields: &Object) -> Result<String, MinimaxSpeechError> {
    let data = data_object(fields)?;
    match data.and_then(|data| data.get("audio")) {
        Some(Value::String(text)) => return Ok(text.clone()),
        Some(Value::Null) | None => {}
        Some(_) => return Err(MinimaxSpeechError::Field { field: "audio" }),
    }
    match fields.get("audio_file") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(MinimaxSpeechError::Field {
            field: "audio_file",
        }),
    }
}

fn no_audio(fields: &Object) -> MinimaxSpeechError {
    let names: Vec<&str> = fields.keys().map(String::as_str).collect();
    MinimaxSpeechError::Reply {
        status: FAILURE_STATUS,
        message: format!("No audio data in MiniMax response. Response keys: {names:?}"),
    }
}

// Python's bytes.fromhex and base64 decoder both ignore ASCII whitespace
fn decode_audio(text: &str) -> Result<Vec<u8>, MinimaxSpeechError> {
    let compact: String = text
        .chars()
        .filter(|letter| !letter.is_ascii_whitespace())
        .collect();
    if let Ok(bytes) = hex::decode(&compact) {
        return Ok(bytes);
    }
    STANDARD
        .decode(compact.as_bytes())
        .map_err(|source| MinimaxSpeechError::Audio { source })
}

fn binary_response(headers: &HeaderMap, audio: Vec<u8>) -> SpeechResponse {
    let mut flat = flatten_headers(headers);
    flat.shift_remove("content-encoding");
    flat.shift_remove("transfer-encoding");
    flat.insert(String::from("content-length"), audio.len().to_string());
    SpeechResponse {
        status: SUCCESS_STATUS,
        content_type: flat.get("content-type").cloned(),
        headers: flat,
        additional_headers: AdditionalHeaders::default(),
        body: stream::once(async move { Ok::<Vec<u8>, HttpError>(audio) }).boxed(),
    }
}
