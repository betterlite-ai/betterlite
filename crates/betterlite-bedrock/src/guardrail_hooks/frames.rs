use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::chunks::StreamChunk;

// LiteLLM's anthropic_sse_error_frames: a failure found after the headers went out can only reach the client as a frame.
#[must_use]
pub fn error_frames(message: &str) -> Vec<StreamChunk> {
    let mut error = BTreeMap::new();
    error.insert(
        "type".to_owned(),
        GuardrailValue::Text("guardrail_error".to_owned()),
    );
    error.insert(
        "message".to_owned(),
        GuardrailValue::Text(message.to_owned()),
    );
    let mut event = BTreeMap::new();
    event.insert("type".to_owned(), GuardrailValue::Text("error".to_owned()));
    event.insert("error".to_owned(), GuardrailValue::Object(error));
    let body = serde_json::to_string(&GuardrailValue::Object(event)).map_or_else(
        |source| {
            tracing::error!("the guardrail error frame could not be serialized: {source}");
            String::new()
        },
        |text| text,
    );
    vec![StreamChunk::Bytes(
        format!("event: error\ndata: {body}\n\n").into_bytes(),
    )]
}
