use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::MessagesFakeStreamError;

fn object(pairs: Vec<(&str, Json)>) -> Json {
    let mut owned_pairs = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        owned_pairs.push((key.to_owned(), value));
    }
    Json::Object(owned_pairs.into_iter().collect())
}

fn typed_object(kind: &str, extra: Vec<(&str, Json)>) -> Json {
    let mut pairs = vec![("type", Json::Text(kind.to_owned()))];
    pairs.extend(extra);
    object(pairs)
}

fn field<'value>(value: &'value Json, key: &str) -> Option<&'value Json> {
    match value {
        Json::Object(fields) => fields.get(key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn indexed_frame(event: &str, index: usize, extra: Vec<(&str, Json)>) -> Vec<u8> {
    let mut pairs = vec![("index", Json::Number(Number::from(index)))];
    pairs.extend(extra);
    format!(
        "event: {event}\ndata: {}\n\n",
        dumps(&typed_object(event, pairs))
    )
    .into_bytes()
}

pub(super) fn content_items(response: &Json) -> Result<&[Json], MessagesFakeStreamError> {
    match field(response, "content") {
        None => Ok(&[]),
        Some(Json::List(items)) => Ok(items),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(MessagesFakeStreamError::ContentNotIterable)
        }
    }
}

fn usage_value(response: &Json) -> Option<&IndexMap<String, Json>> {
    match field(response, "usage") {
        Some(Json::Object(fields)) if !fields.is_empty() => Some(fields),
        _ => None,
    }
}

pub(super) fn message_start(response: &Json) -> Json {
    let usage = usage_value(response);
    let input_tokens = usage
        .and_then(|fields| fields.get("input_tokens"))
        .cloned()
        .unwrap_or_else(|| Json::Number(Number::from(0_i64)));
    let role = field(response, "role")
        .cloned()
        .unwrap_or_else(|| Json::Text("assistant".to_owned()));
    let message = starting_message(response, role, input_tokens);
    typed_object("message_start", vec![("message", message)])
}

fn starting_message(response: &Json, role: Json, input_tokens: Json) -> Json {
    let usage = object(vec![
        ("input_tokens", input_tokens),
        ("output_tokens", Json::Number(Number::from(0_i64))),
    ]);
    object(vec![
        ("id", field(response, "id").cloned().unwrap_or(Json::Null)),
        ("type", Json::Text("message".to_owned())),
        ("role", role),
        (
            "model",
            field(response, "model").cloned().unwrap_or(Json::Null),
        ),
        ("content", Json::List(vec![])),
        ("stop_reason", Json::Null),
        ("stop_sequence", Json::Null),
        ("usage", usage),
    ])
}

fn block_kind(fields: &IndexMap<String, Json>) -> Option<&str> {
    match fields.get("type") {
        Some(Json::Text(value)) => Some(value.as_str()),
        _ => None,
    }
}

pub(super) fn content_block_frames(
    frames: &mut Vec<Vec<u8>>,
    block: &Json,
    index: usize,
) -> Result<(), MessagesFakeStreamError> {
    let Json::Object(fields) = block else {
        return Err(MessagesFakeStreamError::ContentBlockNotMapping);
    };
    match block_kind(fields) {
        Some("text") => {
            let value = fields
                .get("text")
                .cloned()
                .unwrap_or_else(|| Json::Text(String::new()));
            content_block_with_delta(
                frames,
                index,
                ("text", vec![("text", Json::Text(String::new()))]),
                ("text_delta", vec![("text", value)]),
            );
        }
        Some("thinking") => thinking_frames(frames, fields, index),
        Some("redacted_thinking") => {
            block_frames(
                frames,
                index,
                typed_object("redacted_thinking", vec![]),
                None,
            );
        }
        Some("tool_use") => {
            let id = fields.get("id").cloned().unwrap_or(Json::Null);
            let name = fields.get("name").cloned().unwrap_or(Json::Null);
            let input = fields
                .get("input")
                .cloned()
                .unwrap_or_else(|| object(vec![]));
            tool_use_frames(frames, index, id, name, &input);
        }
        Some(_) | None => block_frames(frames, index, block.clone(), None),
    }
    frames.push(indexed_frame("content_block_stop", index, vec![]));
    Ok(())
}

fn block_frames(frames: &mut Vec<Vec<u8>>, index: usize, content_block: Json, delta: Option<Json>) {
    frames.push(indexed_frame(
        "content_block_start",
        index,
        vec![("content_block", content_block)],
    ));
    if let Some(delta_value) = delta {
        frames.push(indexed_frame(
            "content_block_delta",
            index,
            vec![("delta", delta_value)],
        ));
    }
}

fn content_block_with_delta(
    frames: &mut Vec<Vec<u8>>,
    index: usize,
    start: (&str, Vec<(&str, Json)>),
    delta: (&str, Vec<(&str, Json)>),
) {
    let (start_kind, start_extra) = start;
    let (delta_kind, delta_extra) = delta;
    let content_block = typed_object(start_kind, start_extra);
    let delta_value = typed_object(delta_kind, delta_extra);
    block_frames(frames, index, content_block, Some(delta_value));
}

fn tool_use_frames(frames: &mut Vec<Vec<u8>>, index: usize, id: Json, name: Json, input: &Json) {
    let partial_json = dumps(input);
    content_block_with_delta(
        frames,
        index,
        (
            "tool_use",
            vec![("id", id), ("name", name), ("input", object(vec![]))],
        ),
        (
            "input_json_delta",
            vec![("partial_json", Json::Text(partial_json))],
        ),
    );
}

fn thinking_frames(frames: &mut Vec<Vec<u8>>, fields: &IndexMap<String, Json>, index: usize) {
    let content_block = typed_object(
        "thinking",
        vec![
            ("thinking", Json::Text(String::new())),
            ("signature", Json::Text(String::new())),
        ],
    );
    block_frames(frames, index, content_block, None);
    push_thinking_delta(frames, fields, index, "thinking", "thinking_delta");
    push_thinking_delta(frames, fields, index, "signature", "signature_delta");
}

fn push_thinking_delta(
    frames: &mut Vec<Vec<u8>>,
    fields: &IndexMap<String, Json>,
    index: usize,
    name: &str,
    delta_type: &str,
) {
    let value = fields
        .get(name)
        .cloned()
        .unwrap_or_else(|| Json::Text(String::new()));
    if should_emit_delta(&value) {
        let delta = typed_object(delta_type, vec![(name, value)]);
        frames.push(indexed_frame(
            "content_block_delta",
            index,
            vec![("delta", delta)],
        ));
    }
}

fn extra_usage_fields(usage: &IndexMap<String, Json>) -> Vec<(&'static str, Json)> {
    let mut fields = Vec::new();
    for key in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        if let Some(value) = usage.get(key)
            && !matches!(value, Json::Null)
        {
            fields.push((key, value.clone()));
        }
    }
    fields
}

fn stop_delta(response: &Json) -> Json {
    let mut fields = Vec::new();
    for key in ["stop_reason", "stop_sequence"] {
        fields.push((key, field(response, key).cloned().unwrap_or(Json::Null)));
    }
    object(fields)
}

pub(super) fn message_delta(response: &Json) -> Json {
    let usage = usage_value(response);
    let mut delta_fields: Vec<(&str, Json)> = vec![(
        "output_tokens",
        usage
            .and_then(|fields| fields.get("output_tokens"))
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_i64))),
    )];
    if let Some(fields) = usage {
        delta_fields.extend(extra_usage_fields(fields));
    }
    let delta = stop_delta(response);
    typed_object(
        "message_delta",
        vec![("delta", delta), ("usage", object(delta_fields))],
    )
}

pub(super) fn message_stop(response: &Json) -> Json {
    let usage_payload = usage_value(response)
        .cloned()
        .map_or_else(|| object(vec![]), Json::Object);
    typed_object("message_stop", vec![("usage", usage_payload)])
}

fn should_emit_delta(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Json::Text(content) => !content.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
