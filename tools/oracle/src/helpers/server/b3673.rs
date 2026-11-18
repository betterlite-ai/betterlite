use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::anthropic_translation::block_continuation as anthro_block;
use betterlite_server::guardrails::anthropic_translation::sse_events::{
    SseChunk, streaming_has_ended, streaming_scan_key, streaming_string_so_far,
};
use betterlite_server::guardrails::responses_translation::block_events as responses_block;
use serde::Deserialize;
use serde_json::{Value, json};
use std::str;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ResponsesStandalone,
    ResponsesContinuation,
    ResponsesOpenItem,
    ResponsesStreamError,
    AnthropicStandalone,
    AnthropicContinuation,
    AnthropicObservation,
    AnthropicScanKey,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    message: String,
    model: String,
    original_response: Option<Value>,
    stamp: responses_block::BlockStamp,
    events: Vec<Value>,
    chunks: Vec<String>,
    status_code: u16,
    message_id: String,
}

fn decode_frames(chunks: &[Vec<u8>]) -> HelperResult {
    let frames = chunks
        .iter()
        .map(|chunk| {
            let source = str::from_utf8(chunk).or_value_error()?;
            let kind = source.lines().find_map(|line| line.strip_prefix("event: "));
            let data = source
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap_or_default();
            let payload = serde_json::from_str::<Value>(data).or_value_error()?;
            Ok(match kind {
                Some(event) => json!({"event": event, "data": payload}),
                None => payload,
            })
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(json!({"frames": frames}))
}

fn responses_answer(input: &Input, continuation: bool) -> HelperResult {
    let frames = responses_block::build_block_sse_chunks(
        &input.message,
        &input.model,
        input.original_response.as_ref(),
        continuation,
        Some(&input.events),
        &input.stamp,
    )
    .or_value_error()?;
    decode_frames(&frames)
}

fn anthropic_answer(input: &Input, continuation: bool, chunks: &[SseChunk]) -> HelperResult {
    let original = responses_block::guardrail_json::blocked_original_response(
        input.original_response.as_ref(),
    );
    let blocked = anthro_block::BlockedException {
        message: &input.message,
        model: &input.model,
        message_id: &input.message_id,
        original_response: &original,
    };
    let frames = anthro_block::build_block_sse_chunks(&blocked, continuation, Some(chunks))
        .or_value_error()?;
    decode_frames(&frames)
}

fn scan_key_answer(chunks: &[SseChunk]) -> HelperResult {
    let key = streaming_scan_key(chunks);
    let calls = key
        .tool_calls
        .iter()
        .map(|text| serde_json::from_str::<Value>(text))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(json!({"scan_key": {"texts":key.texts,"tool_calls":calls,
        "stream_ended":key.stream_ended,"tool_calls_in_flight":key.tool_calls_in_flight}}))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let chunks: Vec<SseChunk> = input
            .chunks
            .iter()
            .map(|text| SseChunk::Bytes(text.as_bytes().to_vec()))
            .collect();
        match input.operation {
            Operation::ResponsesStandalone => responses_answer(input, false),
            Operation::ResponsesContinuation | Operation::ResponsesOpenItem => {
                responses_answer(input, true)
            }
            Operation::ResponsesStreamError => {
                let events = responses_block::build_stream_error_items(
                    input.status_code,
                    &input.message,
                    Some(&input.events),
                );
                let items: Vec<Value> = events.iter().map(|event| json!({
                    "type": "error", "sequence_number": event.sequence_number, "error": event.error,
                })).collect();
                Ok(json!({"items": items}))
            }
            Operation::AnthropicStandalone => anthropic_answer(input, false, &chunks),
            Operation::AnthropicContinuation => anthropic_answer(input, true, &chunks),
            Operation::AnthropicObservation => Ok(json!({
                "text": streaming_string_so_far(&chunks), "ended": streaming_has_ended(&chunks),
            })),
            Operation::AnthropicScanKey => scan_key_answer(&chunks),
        }
    }
}
