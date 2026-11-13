use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::TranscriptionError;
use crate::api::transcription_contract::TranscriptionResponse;
use crate::openai_base_config::Object;

// The OpenAI SDK derives the reply kind from the request's response_format; json is every other value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedReply {
    pub response: TranscriptionResponse,
    pub original_response: Json,
    // the SDK parsed the body as plain text, not as a JSON object
    pub textual: bool,
}

pub fn reply_format(options: &IndexMap<String, Json>) -> Result<ReplyFormat, TranscriptionError> {
    Ok(match string_field(options, "response_format")? {
        Some("text" | "srt" | "vtt") => ReplyFormat::Text,
        Some(_) | None => ReplyFormat::Json,
    })
}

// decode_transcription_reply is the one place provider reply bytes turn into a TranscriptionResponse.
pub fn decode_transcription_reply(
    body: &[u8],
    content_type: Option<&str>,
    format: ReplyFormat,
) -> Result<DecodedReply, TranscriptionError> {
    if format == ReplyFormat::Json {
        match serde_json::from_slice::<Json>(body) {
            Ok(Json::Object(fields)) => {
                return Ok(DecodedReply {
                    response: response_from_object(&fields)?,
                    original_response: Json::Object(fields),
                    textual: false,
                });
            }
            Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(TranscriptionError::NotObject);
            }
            // the SDK's own branch: the part before any parameter must end in "json"
            Err(source) => {
                let base = content_type.and_then(|kind| kind.split(';').next());
                if base.is_some_and(|kind| kind.trim().ends_with("json")) {
                    return Err(TranscriptionError::Json { source });
                }
            }
        }
    }
    let text = from_utf8(body).map_err(|source| TranscriptionError::Utf8 { source })?;
    Ok(DecodedReply {
        response: TranscriptionResponse {
            text: Some(text.to_owned()),
            extra: Object::new(),
        },
        original_response: Json::Object(IndexMap::from([
            (String::from("text"), Json::Text(text.to_owned())),
            (String::from("usage"), Json::Null),
        ])),
        textual: true,
    })
}

pub fn string_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &str,
) -> Result<Option<&'fields str>, TranscriptionError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(TranscriptionError::Field {
                field: name.to_owned(),
                problem: "is not a string",
            })
        }
    }
}

// convert_to_model_response_object keeps these response fields and a usage whose type is duration or tokens.
fn response_from_object(
    fields: &IndexMap<String, Json>,
) -> Result<TranscriptionResponse, TranscriptionError> {
    let mut extra = Object::new();
    for name in ["language", "task", "duration", "words", "segments"] {
        if let Some(value) = fields.get(name) {
            let kept = serde_json::to_value(value)
                .map_err(|source| TranscriptionError::Json { source })?;
            extra.insert(String::from(name), kept);
        }
    }
    match fields.get("usage") {
        None | Some(Json::Null) => {}
        Some(usage @ Json::Object(parts)) => {
            if matches!(parts.get("type"), Some(Json::Text(kind)) if kind == "duration" || kind == "tokens")
            {
                let kept = serde_json::to_value(usage)
                    .map_err(|source| TranscriptionError::Json { source })?;
                extra.insert(String::from("usage"), kept);
            }
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(TranscriptionError::Field {
                field: String::from("usage"),
                problem: "is not an object",
            });
        }
    }
    Ok(TranscriptionResponse {
        text: string_field(fields, "text")?.map(str::to_owned),
        extra,
    })
}
