use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// LiteLLM's ANTHROPIC_ERROR_STATUS_CODE_MAP, which is not the inverse of its exception mapper
pub const ANTHROPIC_ERROR_STATUS_CODES: [(&str, u16); 8] = [
    ("invalid_request_error", 400),
    ("authentication_error", 401),
    ("permission_error", 403),
    ("not_found_error", 404),
    ("rate_limit_error", 429),
    ("api_error", 500),
    ("overloaded_error", 503),
    ("timeout_error", 504),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChunkFlag {
    MessageStop,
    Ping,
    ContentDelta,
    ProviderError,
    Terminal,
}

#[must_use]
pub fn native_error_status(error_type: &str) -> u16 {
    ANTHROPIC_ERROR_STATUS_CODES
        .iter()
        .find_map(|(name, status)| (*name == error_type).then_some(*status))
        .unwrap_or(500)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagesStreamError {
    pub error_type: String,
    pub message: String,
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMessagesChunk {
    pub flags: BTreeSet<ChunkFlag>,
    pub payloads: Vec<Json>,
}

impl DecodedMessagesChunk {
    #[must_use]
    pub fn new(mut flags: BTreeSet<ChunkFlag>, payloads: Vec<Json>) -> Self {
        let error_payload = payloads.iter().find(|payload| match payload {
            Json::Object(fields) => field_text(fields, "type") == Some("error"),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
        });
        let provider_error = error_payload.and_then(error_body).is_some();
        if provider_error {
            flags.insert(ChunkFlag::ProviderError);
        }
        if flags.contains(&ChunkFlag::MessageStop) || provider_error {
            flags.insert(ChunkFlag::Terminal);
        }
        Self { flags, payloads }
    }

    #[must_use]
    pub fn payload(&self, event_type: &str) -> Option<&Json> {
        self.payloads.iter().find(|payload| match payload {
            Json::Object(fields) => field_text(fields, "type") == Some(event_type),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
        })
    }

    #[must_use]
    pub fn messages_error(&self) -> Option<MessagesStreamError> {
        let body = error_body(self.payload("error")?)?;
        let error_type = field_text(body, "type")?;
        let message = field_text(body, "message").unwrap_or(error_type);
        Some(MessagesStreamError {
            error_type: error_type.to_owned(),
            message: message.to_owned(),
            status: native_error_status(error_type),
        })
    }

    #[must_use]
    pub fn refusal_stop_details(&self) -> Option<&Json> {
        let Json::Object(fields) = self.payload("message_delta")? else {
            return None;
        };
        let Json::Object(delta) = fields.get("delta")? else {
            return None;
        };
        if field_text(delta, "stop_reason") != Some("refusal") {
            return None;
        }
        match delta.get("stop_details") {
            stop_details @ Some(Json::Object(_)) => stop_details,
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => None,
        }
    }
}

fn error_body(payload: &Json) -> Option<&IndexMap<String, Json>> {
    let Json::Object(fields) = payload else {
        return None;
    };
    match fields.get("error") {
        Some(Json::Object(body)) => Some(body),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

pub(super) fn field_text<'value>(
    fields: &'value IndexMap<String, Json>,
    key: &str,
) -> Option<&'value str> {
    match fields.get(key) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
