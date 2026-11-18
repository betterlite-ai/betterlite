use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::AnthropicBlockError;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Event<'message> {
    ContentBlockStop {
        index: i64,
    },
    ContentBlockStart {
        index: i64,
        content_block: Text<'message>,
    },
    ContentBlockDelta {
        index: i64,
        delta: Text<'message>,
    },
    MessageDelta {
        delta: Stop,
        usage: Usage,
    },
    MessageStop,
}

#[derive(Serialize)]
pub(super) struct Text<'message> {
    pub(super) r#type: &'static str,
    pub(super) text: &'message str,
}

#[derive(Serialize)]
pub(super) struct Stop {
    pub(super) stop_reason: &'static str,
    pub(super) stop_sequence: Option<()>,
}

#[derive(Serialize)]
pub(super) struct Usage {
    pub(super) output_tokens: i64,
}

pub(super) fn encode(event: &Event<'_>) -> Result<Vec<u8>, AnthropicBlockError> {
    let kind = match event {
        Event::ContentBlockStop { .. } => "content_block_stop",
        Event::ContentBlockStart { .. } => "content_block_start",
        Event::ContentBlockDelta { .. } => "content_block_delta",
        Event::MessageDelta { .. } => "message_delta",
        Event::MessageStop => "message_stop",
    };
    // Decode the serialized struct directly to preserve field order in the shared encoder.
    let payload: Json = serde_json::from_str(&serde_json::to_string(event)?)?;
    let data = dumps(&payload);
    Ok(format!("event: {kind}\ndata: {data}\n\n").into_bytes())
}
