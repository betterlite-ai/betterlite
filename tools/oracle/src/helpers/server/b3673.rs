use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::anthropic_translation::block_continuation as anthro_block;
use betterlite_server::guardrails::anthropic_translation::sse_events::{
    SseChunk, content_block_state, streaming_has_ended, streaming_scan_key, streaming_string_so_far,
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

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let operation = input
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = input
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let model = input
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let original = input.get("original_response");
        let stamp_value = input.get("stamp").cloned().unwrap_or_else(|| json!({}));
        let stamp: responses_block::BlockStamp =
            serde_json::from_value(stamp_value).or_value_error()?;
        let events = input
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let answer = match operation {
            "responses_standalone" => {
                let chunks = responses_block::build_block_sse_chunks(
                    message, model, original, false, None, &stamp,
                )
                .or_value_error()?;
                response_frames(chunks)?
            }
            "responses_continuation" => {
                let chunks = responses_block::build_block_sse_chunks(
                    message,
                    model,
                    original,
                    true,
                    Some(&events),
                    &stamp,
                )
                .or_value_error()?;
                response_frames(chunks)?
            }
            "responses_open_item" => {
                let chunks = responses_block::build_block_sse_chunks(
                    message,
                    model,
                    original,
                    true,
                    Some(&events),
                    &stamp,
                )
                .or_value_error()?;
                response_frames(chunks)?
            }
            "responses_stream_error" => {
                let code = u16::try_from(
                    input
                        .get("status_code")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                )
                .unwrap_or_default();
                json!({"items": responses_block::build_stream_error_items(code, message, Some(&events))})
            }
            "anthropic_standalone" | "anthropic_continuation" => {
                let chunks: Vec<SseChunk> = input
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
                    .collect();
                let chunks = anthro_block::build_block_sse_chunks(
                    message,
                    model,
                    original,
                    operation == "anthropic_continuation",
                    Some(&chunks),
                    input
                        .get("message_id")
                        .and_then(Value::as_str)
                        .unwrap_or("msg_fixed"),
                )
                .or_value_error()?;
                anthropic_frames(chunks)?
            }
            "anthropic_observation" | "anthropic_scan_key" | "anthropic_content_state" => {
                let chunks: Vec<SseChunk> = input
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
                    .collect();
                match operation {
                    "anthropic_observation" => {
                        json!({"text": streaming_string_so_far(&chunks), "ended": streaming_has_ended(&chunks)})
                    }
                    "anthropic_scan_key" => json!({"scan_key": streaming_scan_key(&chunks)}),
                    _ => {
                        let (open, highest) = content_block_state(&chunks);
                        json!({"open":open,"highest":highest})
                    }
                }
            }
            _ => return Self::not_built("b3673 operation"),
        };
        Ok(answer)
    }
}
