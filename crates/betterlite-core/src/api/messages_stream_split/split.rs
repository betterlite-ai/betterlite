use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk, StreamDelta};

use crate::api::messages_options::refusal::{ChatTurn, chat_refusal_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PayloadKind {
    Reasoning,
    Text,
    ToolCalls,
}

const PAYLOAD_KINDS: [PayloadKind; 3] = [
    PayloadKind::Reasoning,
    PayloadKind::Text,
    PayloadKind::ToolCalls,
];

// The finish split peels finish_reason and usage onto their own chunk, then each payload kind gets one piece
#[must_use]
pub(crate) fn split_chunk(chunk: &StreamChunk) -> Vec<StreamChunk> {
    split_finish(chunk)
        .into_iter()
        .flat_map(split_payloads)
        .collect()
}

fn split_finish(chunk: &StreamChunk) -> Vec<StreamChunk> {
    if !is_combined(chunk) {
        return vec![chunk.clone()];
    }
    let mut content = chunk.clone();
    if let Some(choice) = content.choices.first_mut() {
        choice.finish_reason = None;
    }
    content.usage = None;
    content.hidden_params.remove("usage");
    let mut finish = chunk.clone();
    if let Some(choice) = finish.choices.first_mut() {
        choice.delta.content = None;
        choice.delta.tool_calls = None;
        choice.delta.reasoning_content = None;
        choice.delta.thinking_blocks = None;
    }
    vec![content, finish]
}

fn is_combined(chunk: &StreamChunk) -> bool {
    let Some(choice) = chunk.choices.first() else {
        return false;
    };
    if choice.finish_reason.is_none() {
        return false;
    }
    let delta = &choice.delta;
    payload_kind_present(delta, PayloadKind::Reasoning)
        || payload_kind_present(delta, PayloadKind::Text)
        || payload_kind_present(delta, PayloadKind::ToolCalls)
        || refusal_text(delta).is_some()
}

fn split_payloads(chunk: StreamChunk) -> Vec<StreamChunk> {
    if chunk.choices.len() != 1 {
        return vec![chunk];
    }
    let Some(choice) = chunk.choices.first() else {
        return vec![chunk];
    };
    let delta = &choice.delta;
    if let Some(JsonValue::Array(calls)) = &delta.tool_calls
        && !calls.is_empty()
        && !calls.iter().any(tool_call_has_name)
    {
        return vec![chunk];
    }
    let present: Vec<PayloadKind> = PAYLOAD_KINDS
        .into_iter()
        .filter(|kind| payload_kind_present(delta, *kind))
        .collect();
    if present.len() <= 1 {
        return vec![chunk];
    }
    let role = delta.role.clone();
    let mut pieces = Vec::with_capacity(present.len());
    for (index, kind) in present.into_iter().enumerate() {
        let piece_role = if index == 0 { role.clone() } else { None };
        let mut piece = chunk.clone();
        if let Some(piece_choice) = piece.choices.first_mut() {
            piece_choice.delta = rebuilt_delta(&chunk, kind, piece_role);
        }
        pieces.push(piece);
    }
    pieces
}

fn rebuilt_delta(source: &StreamChunk, kind: PayloadKind, role: Option<String>) -> StreamDelta {
    let Some(choice) = source.choices.first() else {
        return StreamDelta::default();
    };
    let delta = &choice.delta;
    let mut piece = StreamDelta::default();
    match kind {
        PayloadKind::Reasoning => {
            let thinking = delta
                .thinking_blocks
                .clone()
                .filter(|value| matches!(value, JsonValue::Array(items) if !items.is_empty()));
            if let Some(text) = thinking.as_ref().and_then(unsigned_thinking_text) {
                piece.reasoning_content = Some(text);
            } else {
                piece.reasoning_content = delta
                    .reasoning_content
                    .clone()
                    .filter(|text| !text.is_empty());
                piece.thinking_blocks = thinking;
            }
        }
        PayloadKind::Text => {
            piece.content = delta.content.clone().filter(|text| !text.is_empty());
        }
        PayloadKind::ToolCalls => {
            piece.tool_calls = delta
                .tool_calls
                .clone()
                .filter(|value| matches!(value, JsonValue::Array(items) if !items.is_empty()));
        }
    }
    piece.role = role;
    piece
}

// A signature-less thinking block would open a block body with the whole text, so the delta carries it as reasoning
fn unsigned_thinking_text(blocks: &JsonValue) -> Option<String> {
    let JsonValue::Array(items) = blocks else {
        return None;
    };
    let mut text = String::new();
    for block in items {
        let JsonValue::Object(fields) = block else {
            continue;
        };
        if is_present(fields.get("signature")) {
            return None;
        }
        if fields.get("type").and_then(JsonValue::as_str) == Some("thinking")
            && let Some(thinking) = fields.get("thinking").and_then(JsonValue::as_str)
        {
            text.push_str(thinking);
        }
    }
    (!text.is_empty()).then_some(text)
}

fn payload_kind_present(delta: &StreamDelta, kind: PayloadKind) -> bool {
    match kind {
        PayloadKind::Reasoning => {
            matches!(delta.reasoning_content.as_deref(), Some(text) if !text.is_empty())
                || matches!(&delta.thinking_blocks, Some(JsonValue::Array(items)) if !items.is_empty())
        }
        PayloadKind::Text => {
            matches!(delta.content.as_deref(), Some(text) if !text.is_empty())
        }
        PayloadKind::ToolCalls => {
            matches!(&delta.tool_calls, Some(JsonValue::Array(items)) if !items.is_empty())
        }
    }
}

fn tool_call_has_name(tool_call: &JsonValue) -> bool {
    let JsonValue::Object(fields) = tool_call else {
        return false;
    };
    let Some(JsonValue::Object(function)) = fields.get("function") else {
        return false;
    };
    is_present(function.get("name"))
}

fn is_present(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(flag)) => *flag,
        Some(JsonValue::Number(number)) => number.as_f64().is_some_and(|real| real != 0.0_f64),
        Some(JsonValue::String(text)) => !text.is_empty(),
        Some(JsonValue::Array(items)) => !items.is_empty(),
        Some(JsonValue::Object(fields)) => !fields.is_empty(),
    }
}

fn refusal_text(delta: &StreamDelta) -> Option<String> {
    let refusal = delta.extra.get("refusal").map_or(Json::Null, json_of);
    let provider_fields = delta
        .provider_specific_fields
        .as_ref()
        .map_or(Json::Null, |fields| {
            Json::Object(
                fields
                    .iter()
                    .map(|(key, field)| (key.clone(), json_of(field)))
                    .collect(),
            )
        });
    let turn = ChatTurn {
        refusal: &refusal,
        provider_specific_fields: &provider_fields,
    };
    chat_refusal_text(&turn).map(str::to_owned)
}

fn json_of(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(json_of).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), json_of(field)))
                .collect(),
        ),
    }
}
