use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::messages_options::INCOMPLETE_STREAM_MESSAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResponseEventsError {
    #[error("the Messages response must be a JSON object")]
    ResponseNotObject,
    #[error("the Messages response content must be a list")]
    ContentNotList,
    #[error("Messages content block {index} must be a JSON object")]
    BlockNotObject { index: usize },
    #[error("the Messages usage must be a JSON object")]
    UsageNotObject,
}

pub fn response_as_sse_events(response: &Json) -> Result<Vec<Vec<u8>>, ResponseEventsError> {
    let Json::Object(fields) = response else {
        return Err(ResponseEventsError::ResponseNotObject);
    };
    let usage = given_or(fields.get("usage"), Json::Object(IndexMap::new()));
    let Json::Object(usage_fields) = &usage else {
        return Err(ResponseEventsError::UsageNotObject);
    };
    let content = given_or(fields.get("content"), Json::List(Vec::new()));
    let Json::List(blocks) = &content else {
        return Err(ResponseEventsError::ContentNotList);
    };
    let mut frames = vec![message_start(fields, usage_fields)];
    for (index, block) in blocks.iter().enumerate() {
        content_block_events(&mut frames, index, block)?;
    }
    frames.push(message_delta(fields, &usage));
    frames.push(message_stop());
    Ok(frames)
}

#[must_use]
pub fn incomplete_error_sse_event() -> Vec<u8> {
    sse_event(
        "error",
        &object(vec![
            ("type", Json::Text("error".to_owned())),
            (
                "error",
                object(vec![
                    ("type", Json::Text("api_error".to_owned())),
                    ("message", Json::Text(INCOMPLETE_STREAM_MESSAGE.to_owned())),
                ]),
            ),
        ]),
    )
}

fn message_start(fields: &IndexMap<String, Json>, usage: &IndexMap<String, Json>) -> Vec<u8> {
    let mut start_usage = usage.clone();
    start_usage.insert(
        "output_tokens".to_owned(),
        Json::Number(Number::from(0_i64)),
    );
    let mut message = fields.clone();
    message.insert("content".to_owned(), Json::List(Vec::new()));
    message.insert("stop_reason".to_owned(), Json::Null);
    message.insert("stop_sequence".to_owned(), Json::Null);
    message.insert("usage".to_owned(), Json::Object(start_usage));
    sse_event(
        "message_start",
        &object(vec![
            ("type", Json::Text("message_start".to_owned())),
            ("message", Json::Object(message)),
        ]),
    )
}

fn message_delta(fields: &IndexMap<String, Json>, usage: &Json) -> Vec<u8> {
    let mut delta = Vec::with_capacity(2);
    for key in ["stop_reason", "stop_sequence"] {
        let value = fields.get(key).cloned().unwrap_or(Json::Null);
        delta.push((key, value));
    }
    sse_event(
        "message_delta",
        &object(vec![
            ("type", Json::Text("message_delta".to_owned())),
            ("delta", object(delta)),
            ("usage", usage.clone()),
        ]),
    )
}

fn message_stop() -> Vec<u8> {
    sse_event(
        "message_stop",
        &object(vec![("type", Json::Text("message_stop".to_owned()))]),
    )
}

fn content_block_events(
    frames: &mut Vec<Vec<u8>>,
    index: usize,
    block: &Json,
) -> Result<(), ResponseEventsError> {
    let Json::Object(fields) = block else {
        return Err(ResponseEventsError::BlockNotObject { index });
    };
    let (start_block, deltas) = block_start_and_deltas(fields);
    frames.push(indexed_event(
        "content_block_start",
        index,
        vec![("content_block", start_block)],
    ));
    for delta in deltas {
        frames.push(indexed_event(
            "content_block_delta",
            index,
            vec![("delta", delta)],
        ));
    }
    frames.push(indexed_event("content_block_stop", index, Vec::new()));
    Ok(())
}

fn block_start_and_deltas(fields: &IndexMap<String, Json>) -> (Json, Vec<Json>) {
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "tool_use" => tool_use_block(fields),
        Some(Json::Text(kind)) if kind == "thinking" => thinking_block(fields),
        Some(Json::Text(kind)) if kind == "redacted_thinking" => redacted_block(fields),
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => text_block(fields),
    }
}

fn tool_use_block(fields: &IndexMap<String, Json>) -> (Json, Vec<Json>) {
    let input = given_or(fields.get("input"), Json::Object(IndexMap::new()));
    let start = object(vec![
        ("id", fields.get("id").cloned().unwrap_or(Json::Null)),
        ("name", fields.get("name").cloned().unwrap_or(Json::Null)),
        ("input", Json::Object(IndexMap::new())),
        ("type", Json::Text("tool_use".to_owned())),
    ]);
    let delta = object(vec![
        ("partial_json", Json::Text(dumps(&input))),
        ("type", Json::Text("input_json_delta".to_owned())),
    ]);
    (start, vec![delta])
}

fn thinking_block(fields: &IndexMap<String, Json>) -> (Json, Vec<Json>) {
    let mut deltas = vec![object(vec![
        (
            "thinking",
            given_or(fields.get("thinking"), Json::Text(String::new())),
        ),
        ("type", Json::Text("thinking_delta".to_owned())),
    ])];
    if let Some(Json::Text(signature)) = fields.get("signature")
        && !signature.is_empty()
    {
        deltas.push(object(vec![
            ("signature", Json::Text(signature.clone())),
            ("type", Json::Text("signature_delta".to_owned())),
        ]));
    }
    let start = object(vec![
        ("thinking", Json::Text(String::new())),
        ("signature", Json::Text(String::new())),
        ("type", Json::Text("thinking".to_owned())),
    ]);
    (start, deltas)
}

fn redacted_block(fields: &IndexMap<String, Json>) -> (Json, Vec<Json>) {
    let start = object(vec![
        ("type", Json::Text("redacted_thinking".to_owned())),
        ("data", fields.get("data").cloned().unwrap_or(Json::Null)),
    ]);
    (start, Vec::new())
}

fn text_block(fields: &IndexMap<String, Json>) -> (Json, Vec<Json>) {
    let start = object(vec![
        ("type", Json::Text("text".to_owned())),
        ("text", Json::Text(String::new())),
    ]);
    let delta = object(vec![
        ("type", Json::Text("text_delta".to_owned())),
        (
            "text",
            given_or(fields.get("text"), Json::Text(String::new())),
        ),
    ]);
    (start, vec![delta])
}

fn indexed_event(event: &str, index: usize, extra: Vec<(&str, Json)>) -> Vec<u8> {
    let mut pairs = vec![
        ("type", Json::Text(event.to_owned())),
        ("index", Json::Number(Number::from(index))),
    ];
    pairs.extend(extra);
    sse_event(event, &object(pairs))
}

fn sse_event(event: &str, payload: &Json) -> Vec<u8> {
    let data = dumps(payload);
    let frame = format!("event: {event}\ndata: {data}\n\n");
    frame.into_bytes()
}

fn object(pairs: Vec<(&str, Json)>) -> Json {
    let mut fields = IndexMap::with_capacity(pairs.len());
    for (key, value) in pairs {
        fields.insert(key.to_owned(), value);
    }
    Json::Object(fields)
}

// LiteLLM writes `value or default` at these sites, so a blank value takes the default
fn given_or(given: Option<&Json>, fallback: Json) -> Json {
    match given {
        Some(present) if !is_blank(present) => present.clone(),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => fallback,
    }
}

fn is_blank(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !flag,
        Json::Number(number) => number.as_f64().is_none_or(|magnitude| magnitude == 0.0_f64),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}
