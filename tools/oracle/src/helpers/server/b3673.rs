use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::anthropic_translation::block_continuation as anthro_block;
use betterlite_server::guardrails::anthropic_translation::sse_events::{
    SseChunk, streaming_has_ended, streaming_scan_key, streaming_string_so_far,
};
use betterlite_server::guardrails::responses_translation::block_events as responses_block;
use serde_json::{Value, json};

pub(crate) struct Op;

fn response_frames(chunks: Vec<Vec<u8>>) -> HelperResult {
    let frames = chunks
        .iter()
        .map(|chunk| {
            let frame = std::str::from_utf8(chunk).or_value_error()?;
            let payload = frame.strip_prefix("data: ").unwrap_or_default().trim();
            serde_json::from_str::<Value>(payload).or_value_error()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"frames": frames}))
}

fn anthropic_frames(chunks: Vec<Vec<u8>>) -> HelperResult {
    let frames = chunks
        .iter()
        .map(|chunk| {
            let frame = std::str::from_utf8(chunk).or_value_error()?;
            let mut lines = frame.lines();
            let kind = lines
                .next()
                .and_then(|line| line.strip_prefix("event: "))
                .unwrap_or_default();
            let data = lines
                .next()
                .and_then(|line| line.strip_prefix("data: "))
                .unwrap_or_default();
            let data = serde_json::from_str::<Value>(data).or_value_error()?;
            Ok(json!({"event": kind, "data": data}))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(json!({"frames": frames}))
}

fn chunks(input: &Value) -> Vec<SseChunk> {
    input
        .get("chunks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            if let Some(bytes) = item.as_str() {
                SseChunk::Bytes(bytes.as_bytes().to_vec())
            } else if let Some(object) = item.as_object() {
                SseChunk::Event(object.clone())
            } else {
                SseChunk::Other
            }
        })
        .collect()
}

fn text_field<'input>(input: &'input Value, name: &str) -> &'input str {
    input.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn events_field(input: &Value) -> Vec<Value> {
    input
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn responses_answer(input: &Value, continuation: bool) -> HelperResult {
    let stamp = input.get("stamp").cloned().unwrap_or_else(|| json!({}));
    let stamp: responses_block::BlockStamp = serde_json::from_value(stamp).or_value_error()?;
    let events = events_field(input);
    let frames = responses_block::build_block_sse_chunks(
        text_field(input, "message"),
        text_field(input, "model"),
        input.get("original_response"),
        continuation,
        continuation.then_some(events.as_slice()),
        &stamp,
    )
    .or_value_error()?;
    response_frames(frames)
}

fn responses_error_answer(input: &Value) -> HelperResult {
    let status_code = input
        .get("status_code")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let status_code = u16::try_from(status_code).unwrap_or_default();
    let events = events_field(input);
    let items = responses_block::build_stream_error_items(
        status_code,
        text_field(input, "message"),
        Some(&events),
    );
    let items = items
        .iter()
        .map(|item| {
            let mut value = serde_json::to_value(item).or_value_error()?;
            if let Value::Object(fields) = &mut value {
                fields.insert("type".to_owned(), json!("error"));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(json!({"items": items}))
}

fn anthropic_answer(input: &Value, continuation: bool) -> HelperResult {
    let sealed = chunks(input);
    let original =
        responses_block::guardrail_json::blocked_original_response(input.get("original_response"));
    let blocked = anthro_block::BlockedException {
        message: text_field(input, "message"),
        model: text_field(input, "model"),
        message_id: input
            .get("message_id")
            .and_then(Value::as_str)
            .unwrap_or("msg_fixed"),
        original_response: Some(&original),
    };
    anthropic_frames(
        anthro_block::build_block_sse_chunks(&blocked, continuation, Some(&sealed))
            .or_value_error()?,
    )
}

fn observation_answer(input: &Value) -> HelperResult {
    let sealed = chunks(input);
    Ok(json!({
        "text": streaming_string_so_far(&sealed),
        "ended": streaming_has_ended(&sealed),
    }))
}

fn scan_key_answer(input: &Value) -> HelperResult {
    let sealed = chunks(input);
    Ok(json!({"scan_key": streaming_scan_key(&sealed)}))
}

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let operation = text_field(input, "operation");
        match operation {
            "responses_standalone" => responses_answer(input, false),
            "responses_continuation" | "responses_open_item" => responses_answer(input, true),
            "responses_stream_error" => responses_error_answer(input),
            "anthropic_standalone" => anthropic_answer(input, false),
            "anthropic_continuation" => anthropic_answer(input, true),
            "anthropic_observation" => observation_answer(input),
            "anthropic_scan_key" => scan_key_answer(input),
            _ => Self::not_built("b3673 operation"),
        }
    }
}
