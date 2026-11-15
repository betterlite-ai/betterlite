use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use serde_json::from_value;

use super::decode::{SonioxReply, bounded_items};
use super::error::ReplyError;
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::records::tokens::SonioxToken;

pub const STATUS_FIELD: &str = "status";
pub const ERROR_MESSAGE_FIELD: &str = "error_message";
pub const ERROR_TYPE_FIELD: &str = "error_type";
pub const AUDIO_DURATION_FIELD: &str = "audio_duration_ms";
pub const TEXT_FIELD: &str = "text";
pub const TOKENS_FIELD: &str = "tokens";

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionMeta {
    pub status: Option<String>,
    pub error_message: Option<String>,
    pub error_type: Option<String>,
    pub audio_duration_ms: Option<JsonValue>,
    pub raw: Object,
}

#[derive(Debug, Clone)]
pub struct SonioxTranscript {
    pub text: Option<String>,
    pub tokens: Vec<SonioxToken>,
    pub raw: Object,
}

pub fn transcription_meta(reply: &SonioxReply) -> Result<TranscriptionMeta, ReplyError> {
    let object = reply.object()?;
    Ok(TranscriptionMeta {
        status: string_field(object, STATUS_FIELD)?,
        error_message: string_field(object, ERROR_MESSAGE_FIELD)?,
        error_type: string_field(object, ERROR_TYPE_FIELD)?,
        audio_duration_ms: object.get(AUDIO_DURATION_FIELD).cloned(),
        raw: object.clone(),
    })
}

pub fn transcript(reply: &SonioxReply, limits: &ApiLimits) -> Result<SonioxTranscript, ReplyError> {
    let object = reply.object()?;
    let tokens = tokens_at(object, limits)?;
    let text = string_field(object, TEXT_FIELD)?;
    Ok(SonioxTranscript {
        text,
        tokens,
        raw: object.clone(),
    })
}

fn string_field(object: &Object, field: &'static str) -> Result<Option<String>, ReplyError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(text) = value.as_str() else {
        return Err(ReplyError::Shape {
            field,
            expected: "a string or null",
        });
    };
    Ok(Some(text.to_owned()))
}

fn tokens_at(object: &Object, limits: &ApiLimits) -> Result<Vec<SonioxToken>, ReplyError> {
    let Some(value) = object.get(TOKENS_FIELD) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let Some(entries) = value.as_array() else {
        return Err(ReplyError::Shape {
            field: TOKENS_FIELD,
            expected: "a list or null",
        });
    };
    bounded_items(limits, entries.len())?;
    from_value(value.clone()).map_err(|source| ReplyError::Field {
        field: TOKENS_FIELD,
        source,
    })
}
