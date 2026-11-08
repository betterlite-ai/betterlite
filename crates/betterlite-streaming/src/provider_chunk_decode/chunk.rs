use std::str::from_utf8;

use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, StreamChunk, Usage,
};

const NOT_JSON: ProviderChunkError = ProviderChunkError::UnexpectedChunk {
    expected: "a JSON value",
};

// A provider's chunk as the provider crates hand it over, before any reader sees it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ProviderChunk {
    Text(String),
    Json(Box<JsonValue>),
    Generic(Box<GenericChunk>),
    Bytes(Vec<u8>),
}

impl ProviderChunk {
    pub fn text(&self) -> Result<&str, ProviderChunkError> {
        match self {
            Self::Text(text) => Ok(text),
            Self::Bytes(bytes) => from_utf8(bytes).map_err(ProviderChunkError::from),
            Self::Json(_) | Self::Generic(_) => Err(ProviderChunkError::UnexpectedChunk {
                expected: "a text line",
            }),
        }
    }

    pub fn json(&self) -> Result<&JsonValue, ProviderChunkError> {
        let Self::Json(value) = self else {
            return Err(NOT_JSON);
        };
        Ok(value)
    }

    // An OpenAI-shaped chunk is the one a provider sends with a choices list.
    #[must_use]
    pub fn is_openai_chunk(&self) -> bool {
        match self {
            Self::Json(value) => {
                as_object(value).is_some_and(|fields| fields.contains_key("choices"))
            }
            Self::Text(_) | Self::Generic(_) | Self::Bytes(_) => false,
        }
    }

    #[must_use]
    pub fn response_id(&self) -> Option<&str> {
        if !self.is_openai_chunk() {
            return None;
        }
        let Self::Json(value) = self else {
            return None;
        };
        as_object(value)?.get("id").and_then(JsonValue::as_str)
    }

    #[must_use]
    pub fn model_name(&self) -> Option<&str> {
        let Self::Json(value) = self else {
            return None;
        };
        as_object(value)?
            .get("model")
            .and_then(JsonValue::as_str)
            .filter(|model| !model.is_empty())
    }

    pub fn openai_chunk(&self) -> Result<StreamChunk, ProviderChunkError> {
        let JsonValue::Object(fields) = self.json()? else {
            return Err(ProviderChunkError::UnexpectedChunk {
                expected: "a JSON object",
            });
        };
        let mut normalized = fields.clone();
        if normalized
            .get("choices")
            .is_none_or(|value| matches!(value, JsonValue::Null))
        {
            normalized.insert("choices".to_owned(), JsonValue::Array(Vec::new()));
        }
        if let Some(JsonValue::Array(choices)) = normalized.get_mut("choices") {
            for choice in choices {
                normalize_choice(choice);
            }
        }
        if normalized
            .get("hidden_params")
            .is_none_or(|value| matches!(value, JsonValue::Null))
        {
            normalized.insert(
                "hidden_params".to_owned(),
                JsonValue::Object(JsonObject::new()),
            );
        }
        let value = serde_json::to_value(JsonValue::Object(normalized))
            .map_err(ProviderChunkError::from)?;
        serde_json::from_value(value).map_err(ProviderChunkError::from)
    }
}

fn normalize_choice(choice: &mut JsonValue) {
    let JsonValue::Object(fields) = choice else {
        return;
    };
    if fields
        .get("delta")
        .is_none_or(|value| matches!(value, JsonValue::Null))
    {
        fields.insert("delta".to_owned(), JsonValue::Object(JsonObject::new()));
    }
}

// The neutral result of one provider chunk, before the shaper turns it into a stream chunk.
#[derive(Debug, Clone, Default)]
pub struct DecodedChunk {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
    pub tool_use: Option<JsonValue>,
    pub provider_specific_fields: Option<JsonObject>,
    pub original: Option<StreamChunk>,
    pub logprobs: Option<JsonValue>,
    pub tool_calls: Option<JsonValue>,
    pub early_return: bool,
    pub system_fingerprint: Option<Option<String>>,
    pub response_system_fingerprint: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProviderKind {
    Builtin,
    Custom,
}

#[derive(Debug, Clone, Copy)]
pub struct RequestOptions {
    pub send_stream_usage: bool,
    pub is_function_call: bool,
}

// The decoder's own state: what CustomStreamWrapper keeps outside the shaper.
pub struct DecodeState {
    pub model: String,
    pub kind: ProviderKind,
    pub received_finish_reason: Option<String>,
    pub intermittent_finish_reason: Option<String>,
    pub sent_first_chunk: bool,
    pub complete_response: String,
    pub completion_stream: Option<String>,
    pub options: RequestOptions,
    pub repeated_chunk_limit: usize,
}

#[must_use]
pub const fn as_object(value: &JsonValue) -> Option<&JsonObject> {
    if let JsonValue::Object(fields) = value {
        return Some(fields);
    }
    None
}

#[must_use]
pub fn as_array(value: &JsonValue) -> Option<&[JsonValue]> {
    match value {
        JsonValue::Array(items) => Some(items),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Object(_) => None,
    }
}

#[must_use]
pub fn json_text(value: &JsonValue) -> String {
    let Ok(plain) = serde_json::to_value(value) else {
        return String::new();
    };
    serde_json::to_string(&plain).unwrap_or_default()
}
