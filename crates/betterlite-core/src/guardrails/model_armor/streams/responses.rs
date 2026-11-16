use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::assembly::Assembly;
use super::chunk::{Chunk, field};
use super::surface::Surface;
use super::wire::StreamWire;

// Every Responses event whose delta is model output already on its way to the client.
const DELTA_EVENTS: [&str; 6] = [
    "response.output_text.delta",
    "response.refusal.delta",
    "response.reasoning_summary_text.delta",
    "response.function_call_arguments.delta",
    "response.custom_tool_call_input.delta",
    "response.mcp_call_arguments.delta",
];

// What makes two delta events part of one field of the turn rather than two fields that merely streamed next to each other.
const FIELD_ATTRS: [&str; 5] = [
    "type",
    "item_id",
    "output_index",
    "content_index",
    "summary_index",
];

// LiteLLM's _responses_api_response_text: every output item's text, then each tool call it carries.
#[must_use]
pub fn response_text<W: StreamWire>(wire: &W, body: &GuardrailValue) -> String {
    let Some(GuardrailValue::Array(items)) = field(body, "output") else {
        return String::new();
    };
    let mut text = String::new();
    for item in items {
        let parts = wire.output_parts(item);
        for part in parts.texts {
            text.push_str(&part);
        }
        for call in parts.encoded_calls {
            text.push_str(&call);
        }
    }
    text
}

// LiteLLM's _responses_delta_field_texts: one field's deltas join as they streamed, separate fields stay apart.
#[must_use]
pub fn delta_field_texts(chunks: &[Chunk]) -> Vec<String> {
    let mut fields: Vec<([Option<&GuardrailValue>; 5], String)> = Vec::new();
    for chunk in chunks {
        let Some(kind) = chunk.event_type() else {
            continue;
        };
        if !DELTA_EVENTS.contains(&kind) {
            continue;
        }
        let Some(payload) = chunk.payload() else {
            continue;
        };
        let Some(GuardrailValue::Text(delta)) = field(payload, "delta") else {
            continue;
        };
        let key = FIELD_ATTRS.map(|name| field(payload, name));
        match fields.iter_mut().find(|(seen, _text)| *seen == key) {
            Some((_seen, text)) => text.push_str(delta),
            None => fields.push((key, delta.clone())),
        }
    }
    fields.into_iter().map(|(_key, text)| text).collect()
}

// LiteLLM's _streaming_content_to_scan: on a Responses stream the body is scanned first, then every delta field it never repeats.
pub fn content_to_scan<W: StreamWire>(
    wire: &W,
    surface: Surface,
    assembled: &Assembly,
    chunks: &[Chunk],
) -> Result<String, W::Failure> {
    let content = match assembled {
        Assembly::Responses(body) => response_text(wire, body),
        Assembly::Model(body) => wire.model_text(body)?,
        Assembly::Completion(_) => String::new(),
    };
    if surface != Surface::Responses {
        return Ok(content);
    }
    let mut unscanned: Vec<String> = Vec::new();
    for text in delta_field_texts(chunks) {
        if !text.is_empty() && !content.contains(text.as_str()) {
            unscanned.push(text);
        }
    }
    let mut joined = content;
    for text in unscanned {
        if !joined.is_empty() {
            joined.push('\n');
        }
        joined.push_str(&text);
    }
    Ok(joined)
}
