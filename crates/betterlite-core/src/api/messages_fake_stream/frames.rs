use super::MessagesFakeStreamError;
use super::python_json::json_dumps_defaults;
use super::value::Json;

fn object(pairs: Vec<(&str, Json)>) -> Json {
    let mut owned_pairs = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        owned_pairs.push((key.to_owned(), value));
    }
    Json::Object(owned_pairs.into_iter().collect())
}

pub(super) fn frame(event: &str, value: &Json) -> Vec<u8> {
    let mut text = String::from("event: ");
    text.push_str(event);
    text.push_str("\ndata: ");
    text.push_str(&json_dumps_defaults(value));
    text.push_str("\n\n");
    text.into_bytes()
}

fn indexed_frame(event: &str, index: usize, extra: Vec<(&str, Json)>) -> Vec<u8> {
    let mut pairs = vec![("type", Json::from(event)), ("index", Json::from(index))];
    pairs.extend(extra);
    frame(event, &object(pairs))
}

pub(super) fn content_items(response: &Json) -> Result<&[Json], MessagesFakeStreamError> {
    match response.get("content") {
        None => Ok(&[]),
        Some(Json::Array(items)) => Ok(items),
        Some(_) => Err(MessagesFakeStreamError::ContentNotIterable),
    }
}

fn usage_value(response: &Json) -> Option<&Json> {
    let usage = response.get("usage")?;
    usage
        .as_object()
        .is_some_and(|fields| !fields.is_empty())
        .then_some(usage)
}

pub(super) fn message_start(response: &Json) -> Json {
    let usage = usage_value(response);
    let input_tokens = usage.map_or_else(
        || Json::from(0_i64),
        |value| {
            value
                .get("input_tokens")
                .cloned()
                .unwrap_or_else(|| Json::from(0_i64))
        },
    );
    let role = response
        .get("role")
        .map_or_else(|| Json::from("assistant"), Json::clone);
    object(vec![
        ("type", Json::from("message_start")),
        (
            "message",
            object(vec![
                ("id", response.get("id").cloned().unwrap_or(Json::Null)),
                ("type", Json::from("message")),
                ("role", role),
                (
                    "model",
                    response.get("model").cloned().unwrap_or(Json::Null),
                ),
                ("content", Json::Array(vec![])),
                ("stop_reason", Json::Null),
                ("stop_sequence", Json::Null),
                (
                    "usage",
                    object(vec![
                        ("input_tokens", input_tokens),
                        ("output_tokens", Json::from(0_i64)),
                    ]),
                ),
            ]),
        ),
    ])
}

pub(super) fn content_block_frames(
    frames: &mut Vec<Vec<u8>>,
    block: &Json,
    index: usize,
) -> Result<(), MessagesFakeStreamError> {
    if !matches!(block, Json::Object(_)) {
        return Err(MessagesFakeStreamError::ContentBlockNotMapping);
    }
    match block.get("type").and_then(Json::as_str) {
        Some("text") => text_block(frames, block, index),
        Some("thinking") => thinking_block(frames, block, index),
        Some("redacted_thinking") => frames.push(indexed_frame(
            "content_block_start",
            index,
            vec![(
                "content_block",
                object(vec![("type", Json::from("redacted_thinking"))]),
            )],
        )),
        Some("tool_use") => tool_use_block(frames, block, index),
        Some(_) | None => frames.push(indexed_frame(
            "content_block_start",
            index,
            vec![("content_block", block.clone())],
        )),
    }
    frames.push(indexed_frame("content_block_stop", index, vec![]));
    Ok(())
}

fn text_block(frames: &mut Vec<Vec<u8>>, block: &Json, index: usize) {
    frames.push(indexed_frame(
        "content_block_start",
        index,
        vec![(
            "content_block",
            object(vec![("type", Json::from("text")), ("text", Json::from(""))]),
        )],
    ));
    let text = block.get("text").cloned().unwrap_or_else(|| Json::from(""));
    frames.push(indexed_frame(
        "content_block_delta",
        index,
        vec![(
            "delta",
            object(vec![("type", Json::from("text_delta")), ("text", text)]),
        )],
    ));
}

fn thinking_block(frames: &mut Vec<Vec<u8>>, block: &Json, index: usize) {
    frames.push(indexed_frame(
        "content_block_start",
        index,
        vec![(
            "content_block",
            object(vec![
                ("type", Json::from("thinking")),
                ("thinking", Json::from("")),
                ("signature", Json::from("")),
            ]),
        )],
    ));
    push_thinking_delta(frames, block, index, "thinking", "thinking_delta");
    push_thinking_delta(frames, block, index, "signature", "signature_delta");
}

fn push_thinking_delta(
    frames: &mut Vec<Vec<u8>>,
    block: &Json,
    index: usize,
    field: &str,
    delta_type: &str,
) {
    let text = block.get(field).cloned().unwrap_or_else(|| Json::from(""));
    if should_emit_delta(&text) {
        frames.push(indexed_frame(
            "content_block_delta",
            index,
            vec![(
                "delta",
                object(vec![("type", Json::from(delta_type)), (field, text)]),
            )],
        ));
    }
}

fn tool_use_block(frames: &mut Vec<Vec<u8>>, block: &Json, index: usize) {
    frames.push(indexed_frame(
        "content_block_start",
        index,
        vec![(
            "content_block",
            object(vec![
                ("type", Json::from("tool_use")),
                ("id", block.get("id").cloned().unwrap_or(Json::Null)),
                ("name", block.get("name").cloned().unwrap_or(Json::Null)),
                ("input", object(vec![])),
            ]),
        )],
    ));
    let input_data = block
        .get("input")
        .cloned()
        .unwrap_or_else(|| object(vec![]));
    frames.push(indexed_frame(
        "content_block_delta",
        index,
        vec![(
            "delta",
            object(vec![
                ("type", Json::from("input_json_delta")),
                ("partial_json", Json::from(json_dumps_defaults(&input_data))),
            ]),
        )],
    ));
}

fn extra_usage_fields(usage: &Json) -> Vec<(&'static str, Json)> {
    let mut fields = Vec::new();
    for key in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        if let Some(field) = usage.get(key)
            && !matches!(field, Json::Null)
        {
            fields.push((key, field.clone()));
        }
    }
    fields
}

pub(super) fn message_delta(response: &Json) -> Json {
    let usage = usage_value(response);
    let mut delta_fields: Vec<(&str, Json)> = vec![(
        "output_tokens",
        usage
            .and_then(|value| value.get("output_tokens"))
            .cloned()
            .unwrap_or_else(|| Json::from(0_i64)),
    )];
    if let Some(value) = usage {
        delta_fields.extend(extra_usage_fields(value));
    }
    object(vec![
        ("type", Json::from("message_delta")),
        (
            "delta",
            object(vec![
                (
                    "stop_reason",
                    response.get("stop_reason").cloned().unwrap_or(Json::Null),
                ),
                (
                    "stop_sequence",
                    response.get("stop_sequence").cloned().unwrap_or(Json::Null),
                ),
            ]),
        ),
        ("usage", object(delta_fields)),
    ])
}

pub(super) fn message_stop(response: &Json) -> Json {
    let usage_payload = usage_value(response)
        .cloned()
        .unwrap_or_else(|| object(vec![]));
    object(vec![
        ("type", Json::from("message_stop")),
        ("usage", usage_payload),
    ])
}

fn should_emit_delta(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Json::String(text) => !text.is_empty(),
        Json::Array(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
