use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::completion::error::{CompletionError, anthropic_text_error};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyReply {
    Error {
        message: String,
    },
    Completion {
        completion: String,
        stop_reason: Option<String>,
    },
}

// anything but a JSON object is the AnthropicTextError that carries the raw text and the reply status
pub fn decode_legacy_anthropic_reply(
    status_code: u16,
    body: &[u8],
) -> Result<LegacyReply, CompletionError> {
    let Ok(fields) = serde_json::from_slice::<BTreeMap<String, &RawValue>>(body) else {
        let text = str::from_utf8(body).map_err(|source| CompletionError::NotUtf8 { source })?;
        return Err(CompletionError::Upstream(Box::new(anthropic_text_error(
            status_code,
            text.to_owned(),
        ))));
    };
    if let Some(error) = fields.get("error") {
        // a string error gives its own text, any other value the JSON text it was sent as
        let message = serde_json::from_str::<String>(error.get())
            .unwrap_or_else(|_source| error.get().to_owned());
        return Ok(LegacyReply::Error { message });
    }
    let completion = fields
        .get("completion")
        .ok_or(CompletionError::MissingField {
            field: "completion",
        })?;
    let stop_reason = fields
        .get("stop_reason")
        .ok_or(CompletionError::MissingField {
            field: "stop_reason",
        })?;
    Ok(LegacyReply::Completion {
        completion: serde_json::from_str(completion.get()).map_err(|_source| {
            CompletionError::FieldType {
                field: "completion",
                expected: "text",
            }
        })?,
        stop_reason: serde_json::from_str(stop_reason.get()).map_err(|_source| {
            CompletionError::FieldType {
                field: "stop_reason",
                expected: "text or null",
            }
        })?,
    })
}
