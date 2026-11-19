use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::messages_chat_output::output::{AnthropicStopReason, UsageProjection};
use crate::api::messages_options::refusal::refusal_stop_details;

use super::error::MessagesResponseEventError;

fn object(pairs: Vec<(&str, Json)>) -> Json {
    let mut owned = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        owned.push((key.to_owned(), value));
    }
    Json::Object(owned.into_iter().collect())
}

pub(super) fn typed_object(kind: &str, extra: Vec<(&str, Json)>) -> Json {
    let mut pairs = vec![("type", Json::Text(kind.to_owned()))];
    pairs.extend(extra);
    object(pairs)
}

#[must_use]
pub(super) fn message_start(message_id: &str, model: &str) -> Json {
    let zero = Json::Number(Number::from(0_u8));
    let usage = object(vec![
        ("input_tokens", zero.clone()),
        ("output_tokens", zero.clone()),
        ("cache_creation_input_tokens", zero.clone()),
        ("cache_read_input_tokens", zero),
    ]);
    let message = object(vec![
        ("id", Json::Text(message_id.to_owned())),
        ("type", Json::Text("message".to_owned())),
        ("role", Json::Text("assistant".to_owned())),
        ("content", Json::List(Vec::new())),
        ("model", Json::Text(model.to_owned())),
        ("stop_reason", Json::Null),
        ("stop_sequence", Json::Null),
        ("usage", usage),
    ]);
    typed_object("message_start", vec![("message", message)])
}

#[must_use]
pub(super) fn tool_block(call_id: &Json, name: &Json) -> Json {
    typed_object(
        "tool_use",
        vec![
            ("id", call_id.clone()),
            ("name", name.clone()),
            ("input", object(Vec::new())),
        ],
    )
}

#[must_use]
pub(super) fn thinking_block() -> Json {
    typed_object(
        "thinking",
        vec![
            ("thinking", Json::Text(String::new())),
            ("signature", Json::Text(String::new())),
        ],
    )
}

#[must_use]
pub(super) fn redacted_thinking_block(signature: &str) -> Json {
    typed_object(
        "redacted_thinking",
        vec![("data", Json::Text(signature.to_owned()))],
    )
}

// Both a block start and a block delta: the frame type, the block's index and one named payload.
#[must_use]
pub(super) fn block_frame(kind: &str, index: i64, field: &str, payload: Json) -> Json {
    typed_object(
        kind,
        vec![
            ("index", Json::Number(Number::from(index))),
            (field, payload),
        ],
    )
}

#[must_use]
pub(super) fn content_block_stop(index: i64) -> Json {
    typed_object(
        "content_block_stop",
        vec![("index", Json::Number(Number::from(index)))],
    )
}

#[must_use]
pub(super) fn signature_delta(signature: &str) -> Json {
    typed_object(
        "signature_delta",
        vec![("signature", Json::Text(signature.to_owned()))],
    )
}

pub(super) fn message_delta(
    stop_reason: AnthropicStopReason,
    refusal_text: Option<&str>,
    usage: &UsageProjection,
) -> Result<Json, MessagesResponseEventError> {
    let mut delta = vec![
        ("stop_reason", json_from(&stop_reason)?),
        ("stop_sequence", Json::Null),
    ];
    if let Some(text) = refusal_text {
        let details = refusal_stop_details(Some(text.to_owned()));
        delta.push(("stop_details", json_from(&details)?));
    }
    let event = typed_object(
        "message_delta",
        vec![("delta", object(delta)), ("usage", json_from(usage)?)],
    );
    Ok(event)
}

fn json_from<T: serde::Serialize>(value: &T) -> Result<Json, MessagesResponseEventError> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value::<Json>)
        .map_err(MessagesResponseEventError::Json)
}
